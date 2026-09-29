//! The `unsafe` budget (RULES 2): the number of `unsafe` blocks and
//! `unsafe impl`s in the safe crate may not grow without updating
//! `xtask/unsafe-budget` in the same change.
//!
//! The count covers every Rust file of `clingox` (sources and tests), of
//! `clingox-derive`, and the hand-written sources of `clingox-sys`. The
//! generated bindings are left out: they declare functions and contain no
//! blocks.

use std::path::{Path, PathBuf};

use crate::util::{Result, root};

/// Directories whose `.rs` files are counted, relative to the workspace root.
const COUNTED: [&str; 3] = ["clingox", "clingox-derive", "clingox-sys/src"];
/// Files that are never counted.
const EXCLUDED: [&str; 1] = ["clingox-sys/src/bindings.rs"];

pub(crate) fn check() -> Result<()> {
    let text = std::fs::read_to_string(root().join("xtask/unsafe-budget"))?;
    let budget: usize = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .ok_or("xtask/unsafe-budget has no number")?
        .parse()
        .map_err(|e| format!("xtask/unsafe-budget: {e}"))?;

    let mut files = Vec::new();
    for dir in COUNTED {
        collect(&root().join(dir), &mut files)?;
    }
    let mut total = 0;
    let mut per_file = Vec::new();
    for file in &files {
        let relative = file.strip_prefix(root()).unwrap_or(file);
        if EXCLUDED.iter().any(|e| relative == Path::new(e)) {
            continue;
        }
        let count = count_unsafe(&std::fs::read_to_string(file)?);
        if count > 0 {
            per_file.push(format!("  {count:4} {}", relative.display()));
            total += count;
        }
    }
    per_file.sort();
    if total > budget {
        return Err(format!(
            "{total} unsafe blocks and impls, above the budget of {budget} in \
             xtask/unsafe-budget:\n{}",
            per_file.join("\n")
        )
        .into());
    }
    if total < budget {
        eprintln!("check: {total} unsafe blocks and impls, below the budget of {budget}; lower it");
    } else {
        eprintln!("check: {total} unsafe blocks and impls, as budgeted");
    }
    Ok(())
}

/// Every `.rs` file below `dir`, skipping build output.
fn collect(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            collect(&path, files)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
    Ok(())
}

/// Counts `unsafe {` blocks and `unsafe impl`s in Rust source, ignoring
/// comments, string literals and character literals. `unsafe fn` and
/// `unsafe extern` declarations are not counted: their bodies need their own
/// blocks (`unsafe_op_in_unsafe_fn`), which are.
pub(crate) fn count_unsafe(source: &str) -> usize {
    let code = strip(source);
    let bytes = code.as_bytes();
    let mut count = 0;
    let mut from = 0;
    while let Some(found) = code[from..].find("unsafe") {
        let start = from + found;
        let end = start + "unsafe".len();
        from = end;
        let word_before = start > 0 && is_ident(bytes[start - 1]);
        let word_after = end < bytes.len() && is_ident(bytes[end]);
        if word_before || word_after {
            continue;
        }
        let rest = code[end..].trim_start();
        if rest.starts_with('{')
            || rest
                .strip_prefix("impl")
                .is_some_and(|r| !r.starts_with(|c: char| c == '_' || c.is_ascii_alphanumeric()))
        {
            count += 1;
        }
    }
    count
}

fn is_ident(b: u8) -> bool {
    b == b'_' || b.is_ascii_alphanumeric()
}

/// Replaces comments and the contents of string and character literals with
/// spaces, keeping everything else.
fn strip(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            let mut depth = 0;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            out.push(' ');
        } else if c == 'r' && (next == Some('"') || next == Some('#')) && !prev_is_ident(&chars, i)
        {
            // A raw string: r"…", r#"…"#, and so on.
            let mut j = i + 1;
            let mut hashes = 0;
            while chars.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            if chars.get(j) == Some(&'"') {
                j += 1;
                loop {
                    match chars.get(j) {
                        None => break,
                        Some('"') if (1..=hashes).all(|k| chars.get(j + k) == Some(&'#')) => {
                            j += 1 + hashes;
                            break;
                        }
                        Some(_) => j += 1,
                    }
                }
                out.push_str("\"\"");
                i = j;
            } else {
                out.push(c);
                i += 1;
            }
        } else if c == '"' {
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                i += if chars[i] == '\\' { 2 } else { 1 };
            }
            i += 1;
            out.push_str("\"\"");
        } else if c == '\'' {
            // A character literal, or a lifetime, which is kept.
            if next == Some('\\') {
                i += 2;
                while i < chars.len() && chars[i] != '\'' {
                    i += 1;
                }
                i += 1;
                out.push_str("' '");
            } else if chars.get(i + 2) == Some(&'\'') {
                i += 3;
                out.push_str("' '");
            } else {
                out.push(c);
                i += 1;
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

fn prev_is_ident(chars: &[char], i: usize) -> bool {
    i > 0 && (chars[i - 1] == '_' || chars[i - 1].is_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::count_unsafe;

    #[test]
    fn counts_blocks_and_impls() {
        let source = r"
            unsafe impl Send for X {}
            fn f() { let a = unsafe { g() }; unsafe{ h() } }
            call(|| unsafe {
                x()
            });
        ";
        assert_eq!(count_unsafe(source), 4);
    }

    #[test]
    fn ignores_declarations_comments_strings_and_names() {
        let source = r##"
            unsafe fn f() {}
            pub(crate) unsafe extern "C" fn g() {}
            // unsafe { in a comment }
            /* unsafe { in a /* nested */ block } */
            /// unsafe { in a doc comment }
            let s = "unsafe { in a string }";
            let r = r#"unsafe { in a raw string }"#;
            let c = '{'; let l: &'static str = "";
            #![deny(unsafe_code)]
            #[allow(unsafe_code)]
            fn not_unsafe_block() {}
            let unsafe_ish = 1;
        "##;
        assert_eq!(count_unsafe(source), 0);
    }
}
