// `ErrorKind::Interrupted` is gone: interrupts and
// timeouts are search results, never errors.
fn main() {
    let _ = clingox::ErrorKind::Interrupted;
}
