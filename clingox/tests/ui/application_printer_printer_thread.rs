// the default printer holds clasp's stdout lock on the printing thread, so it is `!Send`: a scoped thread cannot receive it (calling it there deadlocks).
// Expected: E0277 (`*mut u8`-like marker: cannot be sent between threads safely).

use clingox::application::Application;

fn main() {
    let _ = Application::new().print_model(|_model, printer| {
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let _ = printer.print();
            });
        });
        Ok(())
    });
}
