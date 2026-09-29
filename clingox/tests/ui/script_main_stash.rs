// A script's `main` receives `&mut ScopedControl<'_>`, whose lifetime is
// universally quantified in the trait method, exactly as in the bound of
// `Application::main`; nothing that would keep the control past the call may
// compile. Swapping an owned control in and stashing the borrowed one is the
// attack. Expected: E0521 borrowed data escapes outside of method.
//

use std::sync::Mutex;

use clingox::ast::Span;
use clingox::script::Script;
use clingox::{Control, Result};

struct Stasher {
    keep: Mutex<Option<Control>>,
}

impl Script for Stasher {
    fn execute(&self, _span: &Span, _code: &str) -> Result<()> {
        Ok(())
    }

    fn main(&self, control: &mut clingox::ScopedControl<'_>) -> Result<()> {
        let owned = std::mem::replace(control, Control::new()?);
        *self.keep.lock().unwrap() = Some(owned);
        Ok(())
    }
}

fn main() {
    let _ = clingox::script::register(
        "stasher",
        "1",
        Stasher {
            keep: Mutex::new(None),
        },
    );
}
