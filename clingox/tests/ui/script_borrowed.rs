// A registered script is leaked for the rest of the process (clingo never
// frees it), so it must be `'static`: a script borrowing a local does not
// compile. Expected: the borrow of `text` must outlive `'static` (E0597).
//

use clingox::ast::Span;
use clingox::script::Script;
use clingox::Result;

struct Borrowing<'a>(&'a str);

impl Script for Borrowing<'_> {
    fn execute(&self, _span: &Span, _code: &str) -> Result<()> {
        let _ = self.0;
        Ok(())
    }
}

fn main() {
    let text = String::from("local");
    let _ = clingox::script::register("borrowing", "1", Borrowing(&text));
}
