// `Ast` must not be `Send`: `AST::refCount_` is a plain, non-atomic `unsigned`
// (astv2.hh:125), incremented and decremented with no lock or atomic
// instruction (astv2.cc:238-243, DESIGN S12), so moving a handle to another
// thread while a clone stays behind and drops concurrently would race on
// that field.
fn assert_send<T: Send>() {}

fn main() {
    assert_send::<clingox::ast::Ast>();
}
