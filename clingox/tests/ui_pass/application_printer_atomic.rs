// What the printer's bound allows: state in atomics and a `Mutex` captured
// by reference, a read of the model, and `print()`. The application may not be
// used after its captures die, but the captures are readable after it is dropped.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use clingox::application::Application;
use clingox::{Model, ShowType};

fn shown(model: &Model) -> clingox::Result<Vec<String>> {
    Ok(model
        .symbols(ShowType::SHOWN)?
        .iter()
        .map(ToString::to_string)
        .collect())
}

fn main() {
    let calls = AtomicUsize::new(0);
    let seen = Mutex::new(Vec::<Vec<String>>::new());
    let app = Application::new()
        .main(|_ctl, _files| Ok(()))
        .print_model(|model, printer| {
            calls.fetch_add(1, Ordering::SeqCst);
            seen.lock().unwrap().push(shown(model)?);
            printer.print()
        });
    drop(app);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(seen.lock().unwrap().is_empty());
}
