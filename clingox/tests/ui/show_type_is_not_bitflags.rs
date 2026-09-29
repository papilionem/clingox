// `ShowType` is clingox's own type, so `bitflags` is not a public dependency
// and no bits outside clingo's flags can be built .
fn main() {
    let _ = clingox::ShowType::from_bits_retain(u32::MAX);
}
