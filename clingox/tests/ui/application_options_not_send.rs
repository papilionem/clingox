// `Options` refers to clingo's option context, which belongs to the thread
// that called `run`: it is neither `Send` nor `Sync`.
// Expected: E0277 (a raw pointer cannot be sent between threads safely).
//

use clingox::application::Options;

fn assert_send<T: Send>() {}

fn main() {
    assert_send::<Options<'static>>();
}
