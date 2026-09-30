//! `cargo xtask package-lists`: the files each published crate ships, against
//! the lists committed in `xtask/package-lists/<crate>.txt`.
//!
//! The manifests name what a package contains with `include` (default deny),
//! and this check catches the rest: a new file matched by a glob, a file that
//! moved out of one, a README or licence Cargo stopped adding. Any change to a
//! list fails `cargo xtask check` until the committed list is updated with
//! `cargo xtask package-lists --bless`, so the change shows in review.
//!
//! The lists are the output of `cargo package --list`, sorted, with forward
//! slashes, and without `.cargo_vcs_info.json`, which Cargo writes only in a
//! git checkout; they are the same on every host.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use crate::util::{Result, cargo, output, root};

/// The published crates.
const CRATES: [&str; 3] = ["clingox-sys", "clingox-derive", "clingox"];

/// Where the expected lists are committed, relative to the workspace root.
const DIR: &str = "xtask/package-lists";

/// Compares every crate's list with the committed one, or with `bless`
/// writes the current lists instead.
pub(crate) fn run(bless: bool) -> Result<()> {
    let mut stale = Vec::new();
    for krate in CRATES {
        let current = list(krate)?;
        let path = root().join(DIR).join(format!("{krate}.txt"));
        if bless {
            std::fs::create_dir_all(root().join(DIR))?;
            std::fs::write(&path, &current)?;
            eprintln!("package-lists: wrote {DIR}/{krate}.txt");
            continue;
        }
        // A checkout with CRLF line endings must compare equal.
        let committed = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {DIR}/{krate}.txt: {e}"))?
            .replace("\r\n", "\n");
        if committed != current {
            stale.push(difference(krate, &committed, &current));
        }
    }
    if !stale.is_empty() {
        return Err(format!(
            "the packaged files changed:\n{}\nIf that is intended, run \
             `cargo xtask package-lists --bless` and commit {DIR}.",
            stale.join("\n")
        )
        .into());
    }
    if !bless {
        eprintln!("check: package lists of {} match {DIR}", CRATES.join(", "));
    }
    Ok(())
}

/// The normalised `cargo package --list` of one crate, one path per line.
fn list(krate: &str) -> Result<String> {
    let raw = output(cargo().args(["package", "--package", krate, "--list", "--allow-dirty"]))?;
    let paths: BTreeSet<String> = raw
        .lines()
        .map(|line| line.trim().replace('\\', "/"))
        .filter(|path| !path.is_empty() && path != ".cargo_vcs_info.json")
        .collect();
    let mut text = String::new();
    for path in paths {
        text.push_str(&path);
        text.push('\n');
    }
    Ok(text)
}

/// The paths added to and removed from one crate's list, one per line.
fn difference(krate: &str, committed: &str, current: &str) -> String {
    let old: BTreeSet<&str> = committed.lines().collect();
    let new: BTreeSet<&str> = current.lines().collect();
    let mut out = format!("  {krate}:");
    for path in new.difference(&old) {
        let _ = write!(out, "\n    + {path}");
    }
    for path in old.difference(&new) {
        let _ = write!(out, "\n    - {path}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::difference;

    #[test]
    fn difference_names_added_and_removed_paths() {
        let text = difference("c", "a\nb\n", "b\nc\n");
        assert_eq!(text, "  c:\n    + c\n    - a");
    }
}
