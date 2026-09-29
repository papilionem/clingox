// The control owns its logger and may outlive any local, so the logger must be
// `'static` and cannot borrow one.
// Expected: E0373, the closure may outlive `main` but borrows `messages`,
// because `logger` requires `'static`.
use clingox::Control;

fn main() {
    let mut messages: Vec<String> = Vec::new();
    let _ctl = Control::builder()
        .logger(|_, text: &str| messages.push(text.to_owned()))
        .build();
}
