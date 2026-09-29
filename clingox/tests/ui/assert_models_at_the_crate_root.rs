// `assert_models!` has one public path, `clingox::testing::assert_models!`
// .
fn main() {
    let models: Vec<clingox::OwnedModel> = Vec::new();
    clingox::assert_models!(models, []);
}
