use std::process::Command;

use crate::util::{self, Result, cargo, output, root};
use crate::{ast_codegen, bindings, budget, conformance, package_lists};

/// The published crate must stay below this size (DESIGN 5.4).
const MAX_PACKAGE_BYTES: u64 = 8 * 1024 * 1024;

/// Path components that must never appear in the published file list: tests,
/// examples, Catch, the apps, and the Python and Lua bindings (DESIGN 5.4).
const FORBIDDEN_COMPONENTS: [&str; 8] = [
    "tests",
    "test",
    "examples",
    "catch",
    "app",
    "libpyclingo",
    "libluaclingo",
    "doc",
];

pub(crate) fn run() -> Result<()> {
    submodule()?;
    stale_bindings()?;
    stale_generated_ast()?;
    util::run(cargo().args(["fmt", "--all", "--", "--check"]))?;
    // RULES 11.6 allows `unwrap` in tests only, so the test targets are
    // checked in a second pass that permits it. clippy's
    // `allow-unwrap-in-tests` covers `#[test]` functions but not the helper
    // functions of an integration test, which are test code too.
    util::run(cargo().args([
        "clippy",
        "--workspace",
        "--lib",
        "--bins",
        "--",
        "-D",
        "warnings",
    ]))?;
    util::run(cargo().args([
        "clippy",
        "--workspace",
        "--tests",
        "--benches",
        "--examples",
        "--",
        "-D",
        "warnings",
    ]))?;
    util::run(cargo().env("RUSTDOCFLAGS", "-D warnings").args([
        "doc",
        "--workspace",
        "--no-deps",
    ]))?;
    // TESTING 8: the rustdoc examples of every library crate, and through
    // `clingox/src/doctests.rs` every Rust block of the guide and the README,
    // run against the real crates. They run on the host only.
    util::run(cargo().args(["test", "--doc", "--workspace"]))?;
    util::run(cargo().args(["deny", "check"]))?;
    conformance::run()?;
    budget::check()?;
    crate::semver::check_step()?;
    util::run(
        Command::new("mdbook")
            .arg("build")
            .arg(root().join("guide")),
    )?;
    package()?;
    package_lists::run(false)?;
    eprintln!("check: all checks passed");
    Ok(())
}

/// The vendored source must be exactly the pinned upstream commit, and the tag
/// must contain no nested submodules (DESIGN 5.1, RULES 8).
fn submodule() -> Result<()> {
    let clingo = root().join("clingox-sys/clingo");
    let pinned = std::fs::read_to_string(root().join("clingox-sys/clingo.commit"))?;
    let pinned = pinned.trim();

    let head = output(
        Command::new("git")
            .arg("-C")
            .arg(&clingo)
            .args(["rev-parse", "HEAD"]),
    )?;
    if head.trim() != pinned {
        return Err(format!(
            "clingox-sys/clingo is at {}, but clingo.commit pins {pinned}",
            head.trim()
        )
        .into());
    }

    let index = output(Command::new("git").current_dir(root()).args([
        "ls-files",
        "--stage",
        "clingox-sys/clingo",
    ]))?;
    let recorded = index.split_whitespace().nth(1).unwrap_or_default();
    if recorded != pinned {
        return Err(format!(
            "the superproject records clingox-sys/clingo at {recorded:?}, not {pinned}"
        )
        .into());
    }

    let status = output(
        Command::new("git")
            .arg("-C")
            .arg(&clingo)
            .args(["status", "--porcelain"]),
    )?;
    if !status.trim().is_empty() {
        return Err(format!("the vendored clingo source is modified:\n{status}").into());
    }

    let tree = output(
        Command::new("git")
            .arg("-C")
            .arg(&clingo)
            .args(["ls-tree", "-r", "HEAD"]),
    )?;
    let gitlinks: Vec<&str> = tree.lines().filter(|l| l.starts_with("160000 ")).collect();
    if !gitlinks.is_empty() {
        return Err(format!(
            "the pinned clingo tree contains gitlinks, so a plain checkout is incomplete:\n{}",
            gitlinks.join("\n")
        )
        .into());
    }
    eprintln!("check: clingox-sys/clingo is {pinned}, clean, without gitlinks");
    Ok(())
}

fn stale_bindings() -> Result<()> {
    let fresh = bindings::generate()?;
    let committed = std::fs::read_to_string(bindings::output())?;
    if fresh != committed {
        return Err("clingox-sys/src/bindings.rs is stale; run `cargo xtask bindgen`".into());
    }
    eprintln!("check: bindings.rs matches clingo.h");
    Ok(())
}

/// The generated AST layer must be exactly what the generator writes now from
/// clingo's constructor table. Writes nothing.
fn stale_generated_ast() -> Result<()> {
    let fresh = ast_codegen::generate()?;
    for (path, generated) in [
        (ast_codegen::safe_output(), &fresh.safe),
        (ast_codegen::raw_output(), &fresh.raw),
    ] {
        let relative = path
            .strip_prefix(root())
            .unwrap_or(&path)
            .display()
            .to_string();
        let committed = std::fs::read_to_string(&path).map_err(|e| {
            format!("{relative} cannot be read ({e}); run `cargo xtask ast-codegen`")
        })?;
        if &committed != generated {
            return Err(format!("{relative} is stale; run `cargo xtask ast-codegen`").into());
        }
    }
    eprintln!("check: generated AST layer matches the constructor table");
    Ok(())
}

fn package() -> Result<()> {
    let list = output(cargo().args([
        "package",
        "--package",
        "clingox-sys",
        "--list",
        "--allow-dirty",
    ]))?;
    let forbidden: Vec<&str> = list
        .lines()
        .filter(|path| path.split('/').any(|c| FORBIDDEN_COMPONENTS.contains(&c)))
        .collect();
    if !forbidden.is_empty() {
        return Err(format!(
            "the package includes excluded files:\n{}",
            forbidden.join("\n")
        )
        .into());
    }
    for required in [
        "build.rs",
        "src/bindings.rs",
        "THIRD-PARTY-LICENSES",
        "clingo/libclingo/clingo.h",
    ] {
        if !list.lines().any(|path| path == required) {
            return Err(format!("the package is missing {required}").into());
        }
    }

    util::run(cargo().args([
        "package",
        "--package",
        "clingox-sys",
        "--no-verify",
        "--allow-dirty",
    ]))?;
    let package_dir = root().join("target/package");
    let newest = std::fs::read_dir(&package_dir)?
        .filter_map(std::result::Result::ok)
        .filter(|entry| {
            let path = entry.path();
            // cargo names the file itself, always in lower case.
            let is_crate = path.extension().is_some_and(|ext| ext == "crate");
            is_crate
                && entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("clingox-sys-")
        })
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .max()
        .map(|(_, path)| path)
        .ok_or("cargo package wrote no clingox-sys .crate file")?;
    let size = std::fs::metadata(&newest)?.len();
    if size > MAX_PACKAGE_BYTES {
        return Err(format!(
            "{} is {size} bytes, above the {MAX_PACKAGE_BYTES}-byte limit",
            newest.display()
        )
        .into());
    }
    eprintln!(
        "check: package has {} files, {}.{:02} MB compressed",
        list.lines().count(),
        size / 1_048_576,
        size % 1_048_576 * 100 / 1_048_576
    );
    Ok(())
}
