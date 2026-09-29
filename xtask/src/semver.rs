//! `cargo xtask semver`: `cargo semver-checks` for the published crates
//! against a git revision (DESIGN 10).
//!
//! The release type is fixed to `minor`, so a breaking change is reported
//! whatever the pre-release version strings say. The baseline is
//! `--baseline-rev <rev>` when given, otherwise the tag named in
//! `xtask/semver-baseline`.
//!
//! The baseline is extracted with `git archive`, which leaves out the clingo
//! submodule, so the submodule of this checkout is linked into it when both
//! pin the same commit.
//!
//! **Expected findings.** A change that is breaking for the tool but
//! source-compatible by design is listed in `xtask/semver-allow`, one entry
//! per finding: the lint name and the item path, then a comment line with the
//! reason. The tool's output is parsed; the step fails on a finding that is not
//! listed (its output is shown verbatim above the error) and on a listed entry
//! that no longer occurs, so stale entries are removed. Nothing is allowed
//! crate-wide. Both sides build their docs with `DOCS_RS=1`, which
//! skips the native build: the API does not depend on it, and clingo is then
//! not compiled twice more.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::util::{Result, cargo, output, root};

/// The crates checked, as `-p` arguments. `cargo semver-checks` 0.50 skips
/// `clingox-derive` without a message, because rustdoc gives it no API data
/// for a proc-macro crate; its interface is pinned by the trybuild tests.
const CRATES: [&str; 6] = ["-p", "clingox", "-p", "clingox-sys", "-p", "clingox-derive"];

/// The file that names the release the API is checked against.
const BASELINE_FILE: &str = "xtask/semver-baseline";

/// The expected findings, see the module documentation.
const ALLOW_FILE: &str = "xtask/semver-allow";

pub(crate) fn run(baseline: Option<&str>) -> Result<()> {
    let baseline = match baseline {
        Some(rev) => rev.to_owned(),
        None => baseline_tag()?,
    };
    check(&baseline)
}

/// The step of `cargo xtask check`: the check against the tag in
/// `xtask/semver-baseline`, or a note while that tag does not exist yet.
pub(crate) fn check_step() -> Result<()> {
    let tag = baseline_tag()?;
    if !tag_exists(&tag) {
        eprintln!(
            "check: semver skipped: the baseline tag {tag} does not exist yet \
             (it is created when the first alpha is tagged)"
        );
        return Ok(());
    }
    check(&tag)
}

fn check(baseline: &str) -> Result<()> {
    let dir = extract(baseline)?;
    let mut command = cargo();
    command
        .env("DOCS_RS", "1")
        .args(["semver-checks", "check-release", "--release-type", "minor"])
        .arg("--baseline-root")
        .arg(&dir)
        .args(CRATES);
    eprintln!("$ {command:?}");
    let result = command.output();
    std::fs::remove_dir_all(&dir)?;
    let result = result?;
    // The tool's own output, verbatim.
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    eprint!("{text}");
    let allowed = parse_allow(&std::fs::read_to_string(root().join(ALLOW_FILE))?);
    let found = findings(&text);
    if result.status.success() && found.is_empty() {
        return settle(&allowed, &found);
    }
    if found.is_empty() {
        // Failed without a finding: a build error or a missing tool.
        return Err(format!(
            "semver-checks failed ({}); `cargo install cargo-semver-checks --locked` \
             installs the tool",
            result.status
        )
        .into());
    }
    settle(&allowed, &found)
}

/// Compares the findings with the allow-list: an unlisted finding and a stale
/// entry both fail.
fn settle(allowed: &[(String, String)], found: &[(String, String)]) -> Result<()> {
    let unexpected: Vec<String> = found
        .iter()
        .filter(|f| !allowed.contains(f))
        .map(|(lint, item)| format!("{lint} {item}"))
        .collect();
    let stale: Vec<String> = allowed
        .iter()
        .filter(|a| !found.contains(a))
        .map(|(lint, item)| format!("{lint} {item}"))
        .collect();
    if !unexpected.is_empty() {
        return Err(format!(
            "semver: findings not listed in {ALLOW_FILE}:\n  {}",
            unexpected.join("\n  ")
        )
        .into());
    }
    if !stale.is_empty() {
        return Err(format!(
            "semver: entries of {ALLOW_FILE} that no longer occur (remove them):\n  {}",
            stale.join("\n  ")
        )
        .into());
    }
    eprintln!(
        "check: semver: {} expected finding(s), as listed",
        found.len()
    );
    Ok(())
}

/// The entries of the allow file: `lint item` per line, `#` lines are comments.
fn parse_allow(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let (lint, item) = l.split_once(char::is_whitespace)?;
            Some((lint.to_owned(), item.trim().to_owned()))
        })
        .collect()
}

/// The findings in the output of `cargo semver-checks`, as (lint, item path):
/// each `--- failure <lint>: ...` heading is followed by a `Failed in:` block
/// whose lines read `  <kind> <path>, previously in file ...`.
fn findings(output: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut lint = None;
    let mut in_block = false;
    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("--- failure ") {
            lint = rest.split(':').next().map(str::to_owned);
            in_block = false;
        } else if line.trim() == "Failed in:" {
            in_block = true;
        } else if in_block && line.starts_with("  ") && line.contains(", previously in ") {
            let path = line.trim().split_once(' ').map_or("", |(_, rest)| rest);
            let path = path.split(',').next().unwrap_or("").trim();
            if let (Some(lint), false) = (&lint, path.is_empty()) {
                found.push((lint.clone(), path.to_owned()));
            }
        } else if in_block && line.trim().is_empty() {
            in_block = false;
        }
    }
    found
}

/// Extracts the tree of `rev` into a directory under `target/`, with the
/// clingo submodule of this checkout linked in, and returns the directory.
fn extract(rev: &str) -> Result<PathBuf> {
    let commit = output(
        Command::new("git")
            .current_dir(root())
            .args(["rev-parse", "--verify"])
            .arg(format!("{rev}^{{commit}}")),
    )?;
    let commit = commit.trim();
    let dir = root().join("target/semver-baseline").join(commit);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::create_dir_all(&dir)?;
    let archive = Command::new("git")
        .current_dir(root())
        .args(["archive", "--format=tar", commit])
        .stdout(Stdio::piped())
        .spawn()?;
    let stdout = archive.stdout.ok_or("git archive gave no output")?;
    let status = Command::new("tar")
        .arg("-x")
        .arg("-C")
        .arg(&dir)
        .stdin(stdout)
        .status()?;
    if !status.success() {
        return Err(format!("cannot extract {rev} into {}", dir.display()).into());
    }
    link_submodule(&dir)?;
    Ok(dir)
}

/// Links this checkout's clingo submodule into the baseline, which must pin
/// the same commit: `git archive` leaves submodules out.
fn link_submodule(dir: &Path) -> Result<()> {
    let pinned = |base: &Path| std::fs::read_to_string(base.join("clingox-sys/clingo.commit"));
    let (ours, theirs) = (pinned(&root())?, pinned(dir)?);
    if ours.trim() != theirs.trim() {
        return Err(format!(
            "the baseline pins clingo {}, this checkout {}; a baseline for another \
             clingo needs its own checkout of that submodule",
            theirs.trim(),
            ours.trim()
        )
        .into());
    }
    let link = dir.join("clingox-sys/clingo");
    if link.exists() {
        std::fs::remove_dir(&link)?;
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(root().join("clingox-sys/clingo"), &link)?;
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(root().join("clingox-sys/clingo"), &link)?;
    Ok(())
}

/// The tag in `xtask/semver-baseline`: its first line that is neither empty
/// nor a comment.
fn baseline_tag() -> Result<String> {
    let text = std::fs::read_to_string(root().join(BASELINE_FILE))?;
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_owned)
        .ok_or_else(|| format!("{BASELINE_FILE} names no tag").into())
}

fn tag_exists(tag: &str) -> bool {
    output(
        Command::new("git")
            .current_dir(root())
            .args(["rev-parse", "--verify", "--quiet"])
            .arg(format!("refs/tags/{tag}")),
    )
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real output of this tool for one API change, captured once.
    const SAMPLE: &str = include_str!("semver_sample.txt");

    #[test]
    fn findings_are_read_from_the_tools_output() {
        assert_eq!(
            findings(SAMPLE),
            [
                (
                    "struct_missing".to_owned(),
                    "clingox::prelude::Control".to_owned()
                ),
                ("struct_missing".to_owned(), "clingox::Control".to_owned()),
            ]
        );
        assert!(findings("Summary no semver update required\n").is_empty());
    }

    #[test]
    fn allow_entries_skip_comments_and_settle_exactly() {
        let allowed = parse_allow(
            "# reason\nstruct_missing clingox::Control\n\nstruct_missing clingox::prelude::Control\n",
        );
        let found = findings(SAMPLE);
        assert!(settle(&allowed, &found).is_ok());
        assert!(settle(&allowed[..1], &found).is_err(), "unlisted finding");
        assert!(settle(&allowed, &found[..1]).is_err(), "stale entry");
    }
}
