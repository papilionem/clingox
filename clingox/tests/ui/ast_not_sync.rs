// `Ast` must not be `Sync`, for the same reason it must not be `Send`
// (astv2.hh:125, astv2.cc:238-243, DESIGN S12): two threads cloning or
// dropping `&Ast` handles to the same node at once would race on the
// non-atomic refcount.
fn assert_sync<T: Sync>() {}

fn main() {
    assert_sync::<clingox::ast::Ast>();
}
