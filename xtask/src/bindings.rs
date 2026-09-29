use std::path::PathBuf;

use bindgen::callbacks::ParseCallbacks;
use bindgen::{Formatter, RustEdition, RustTarget};

use crate::util::{Result, msrv, root};

pub(crate) fn header() -> PathBuf {
    root().join("clingox-sys/clingo/libclingo/clingo.h")
}

pub(crate) fn output() -> PathBuf {
    root().join("clingox-sys/src/bindings.rs")
}

/// Generates the bindings in memory, so `check` can compare without writing.
pub(crate) fn generate() -> Result<String> {
    let (minor, patch) = msrv()?;
    let target = RustTarget::stable(minor, patch)
        .map_err(|e| format!("bindgen rust target 1.{minor}.{patch}: {e}"))?;
    let bindings = bindgen::Builder::default()
        .header(header().to_string_lossy())
        // A static build defines this publicly (DESIGN 5.2); without it Windows
        // declarations would be dllimport.
        .clang_arg("-DCLINGO_NO_VISIBILITY")
        .rust_target(target)
        .rust_edition(RustEdition::Edition2024)
        .allowlist_item("clingo_.*")
        .allowlist_item("g_clingo_.*")
        .allowlist_item("CLINGO_.*")
        // The header's string macro would clash with the `(u32, u32, u32)` constant
        // that lib.rs builds from the three numeric macros (DESIGN 5.4).
        .blocklist_item("CLINGO_VERSION")
        // Enum constants keep their C names, and clingo passes enums as int or
        // unsigned typedefs, so bindgen's default constant style fits.
        .prepend_enum_name(false)
        // Layout assertions would pin the host's pointer width into a file that
        // must compile on wasm32 too; systest checks layouts per target instead.
        .layout_tests(false)
        // prettyplease comes from Cargo.lock, so the output does not depend on the
        // rustfmt binary installed on the machine.
        .formatter(Formatter::Prettyplease)
        .parse_callbacks(Box::new(DoxygenComments))
        .generate()
        .map_err(|e| format!("bindgen failed: {e}"))?;
    Ok(bindings.to_string())
}

/// clingo.h writes Doxygen comments as `//!` and `//!<`. bindgen keeps the `!` and
/// `!<` markers, which would show up as text at the start of every doc line.
#[derive(Debug)]
struct DoxygenComments;

impl ParseCallbacks for DoxygenComments {
    fn process_comment(&self, comment: &str) -> Option<String> {
        let mut in_fence = false;
        let lines: Vec<String> = comment
            .lines()
            .map(|line| {
                line.strip_prefix("!<")
                    .or_else(|| line.strip_prefix('!'))
                    .unwrap_or(line)
            })
            .map(|line| {
                // Doxygen fences look like `~~~{.c}`. rustdoc would compile the body
                // as a Rust doctest, so they become fences with a non-Rust language.
                let Some(rest) = line.trim_start().strip_prefix("~~~") else {
                    return line.to_owned();
                };
                in_fence = !in_fence;
                if !in_fence {
                    return " ```".to_owned();
                }
                let lang = rest
                    .trim_start_matches('~')
                    .trim()
                    .trim_start_matches("{.")
                    .trim_end_matches('}');
                format!(" ```{}", if lang.is_empty() { "text" } else { lang })
            })
            .collect();
        Some(lines.join("\n"))
    }
}

pub(crate) fn write() -> Result<()> {
    std::fs::write(output(), generate()?)?;
    eprintln!("wrote {}", output().display());
    Ok(())
}
