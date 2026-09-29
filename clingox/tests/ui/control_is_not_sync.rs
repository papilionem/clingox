// A Control must not be shared between threads (DESIGN S12).
fn assert_sync<T: Sync>() {}

fn main() {
    assert_sync::<clingox::Control>();
}
