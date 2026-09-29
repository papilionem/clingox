// The closure is `Fn` (Q11): it cannot assign to a captured local; state goes in atomics or a `Mutex`.
// Expected: E0594 (cannot assign to `n`, as it is a captured variable in a `Fn` closure).

use clingox::application::Application;

fn main() {
    let mut n = 0;
    let _ = Application::new().print_model(|_model, printer| {
        n += 1;
        printer.print()
    });
}
