// The printer closure runs on solver threads, so it must be `Sync`; a `Cell` is not.
// Expected: E0277 (`Cell<i32>` cannot be shared between threads safely).

use clingox::application::Application;

fn main() {
    let count = std::cell::Cell::new(0);
    let _ = Application::new().print_model(|_model, printer| {
        count.set(count.get() + 1);
        printer.print()
    });
}
