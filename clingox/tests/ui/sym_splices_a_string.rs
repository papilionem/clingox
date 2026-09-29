// A Rust string is either a clingo string or a constant, so `String`
// implements no `ToSymbol`. Expected: the
// unsatisfied bound `String: ToSymbol` at the splice, with the trait's
// `on_unimplemented` note naming `Symbol::string` and `Symbol::function`.
#![forbid(unsafe_code)]

fn main() {
    let name = String::from("comp13");
    let _ = clingox::sym!(p({ name }));
}
