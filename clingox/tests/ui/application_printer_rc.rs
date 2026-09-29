// The printer closure must be `Send` and `Sync`; an `Rc` is neither.
// Expected: E0277 (`Rc<i32>` cannot be shared between threads safely).

use clingox::application::Application;

fn main() {
    let shared = std::rc::Rc::new(1);
    let _ = Application::new().print_model(|_model, printer| {
        let _keep = &shared;
        printer.print()
    });
}
