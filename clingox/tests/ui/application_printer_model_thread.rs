// `&Model` is not `Sync`, so it cannot be handed to another thread.
// Expected: E0277 (cannot be shared between threads safely).

use clingox::Model;
use clingox::application::Application;

fn main() {
    let _ = Application::new().print_model(|model, printer| {
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let _seen: &Model = model;
            });
        });
        printer.print()
    });
}
