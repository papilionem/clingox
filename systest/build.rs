//! Generates the ABI checks for clingox-sys against the real `clingo.h`
//! (DESIGN 5.3): every function signature, struct layout and constant in
//! `bindings.rs` is compared with what a C compiler sees in the header.

use std::env;
use std::path::PathBuf;

fn main() {
    let include = env::var("DEP_CLINGO_INCLUDE").expect("clingox-sys exports DEP_CLINGO_INCLUDE");
    let sys = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../clingox-sys");
    let bindings = std::fs::read_to_string(sys.join("src/bindings.rs"))
        .expect("clingox-sys/src/bindings.rs is readable");
    println!("cargo::rerun-if-changed=../clingox-sys/src");

    // Opaque handles are declared incomplete in C, so neither they nor their
    // typedefs have a size to compare.
    let opaque = opaque_structs(&bindings);
    let opaque_aliases = aliases_of(&bindings, &opaque);

    let mut cfg = ctest::TestGenerator::new();
    // Generate code for the edition systest itself is compiled with.
    cfg.edition(2024)
        .header("clingo.h")
        .include(&include)
        // The same define the static build and bindgen use (DESIGN 5.2).
        .define("CLINGO_NO_VISIBILITY", None)
        // clingo declares enums as `enum clingo_x_e { ... }` without a typedef and
        // passes them as separate int typedefs, so the `_e` types need the tag.
        // ctest 0.5.1 maps a constant's type twice, so an already tagged name
        // must be left alone (alias_is_c_enum would write `enum enum`).
        .rename_type(|ty| {
            (ty.ends_with("_e") && !ty.starts_with("enum ")).then(|| format!("enum {ty}"))
        })
        .skip_struct(move |s| opaque.iter().any(|o| o == s.ident()))
        .skip_alias(move |a| opaque_aliases.iter().any(|o| o == a.ident()))
        // bindgen appends `_` to C field names that are Rust keywords.
        .rename_struct_field(|_, field| (field.ident() == "type_").then(|| "type".to_owned()))
        // A Rust tuple built from the three version macros, which are checked
        // on their own; C has no counterpart.
        .skip_const(|c| c.ident() == "CLINGO_VERSION");
    if env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc") {
        // MSVC gives every C enum the type `int`, where GCC and clang give an
        // enum without negative values `unsigned int`, which is what bindgen saw
        // and `bindings.rs` declares. Both are 32 bits wide and clingo's values
        // are small, so the enums pass and return alike; size and alignment are
        // still compared.
        cfg.skip_signededness(|ty| ty.ends_with("_e"));
    }
    ctest::generate_test(&mut cfg, sys.join("src/lib.rs"), "all.rs")
        .expect("ctest generates the ABI checks");
}

/// Names of the structs bindgen emitted as opaque (`_unused: [u8; 0]`).
fn opaque_structs(bindings: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut current = None;
    for line in bindings.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("pub struct ") {
            current = rest.split([' ', '{']).next().map(str::to_owned);
        } else if line.starts_with("_unused: [u8; 0]")
            && let Some(name) = current.take()
        {
            names.push(name);
        }
    }
    names
}

/// Names of the `pub type A = B;` aliases whose target is one of `targets`.
fn aliases_of(bindings: &str, targets: &[String]) -> Vec<String> {
    bindings
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("pub type ")?
                .strip_suffix(';')?
                .split_once(" = ")
        })
        .filter(|(_, target)| targets.iter().any(|t| t == target))
        .map(|(alias, _)| alias.to_owned())
        .collect()
}
