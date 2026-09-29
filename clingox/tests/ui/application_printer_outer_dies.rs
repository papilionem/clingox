// The application borrows what its closure captures (`'a`) and cannot outlive it.
// Expected: E0597 (`counter` does not live long enough).

use clingox::application::Application;

fn main() {
    let app;
    {
        let counter = std::sync::Mutex::new(0_u32);
        app = Application::new().print_model(|_model, printer| {
            *counter.lock().unwrap() += 1;
            printer.print()
        });
    }
    drop(app);
}
