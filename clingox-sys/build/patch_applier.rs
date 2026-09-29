//! The build-time patch applier (RULES 8): parses a unified diff as `git diff`
//! writes it, and applies it to one file's text.
//!
//! A separate module (not inline in `build.rs`) so
//! `clingox-sys/tests/patches.rs` can include it too and unit-test it directly,
//! without going through a full vendored build.

/// The changes a unified diff makes to one file.
pub(crate) struct FilePatch {
    /// The path relative to the clingo root, from the `+++ b/<path>` line.
    pub(crate) path: String,
    hunks: Vec<Hunk>,
}

/// One `@@` section of a unified diff.
struct Hunk {
    /// The `@@ ... @@` line, for error messages.
    header: String,
    /// The first line of the original the hunk covers, counted from 1.
    old_start: usize,
    /// The hunk's lines, each with its marker: ' ', '-' or '+'.
    lines: Vec<(char, String)>,
}

/// Parses a unified diff as `git diff` writes it. Text before the first file
/// header, such as the patch's own description, is ignored, as `git apply`
/// does.
pub(crate) fn parse_patch(text: &str) -> Result<Vec<FilePatch>, String> {
    let mut files = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let Some(old) = line.strip_prefix("--- ") else {
            continue;
        };
        let new = lines
            .next()
            .and_then(|l| l.strip_prefix("+++ "))
            .ok_or_else(|| format!("`--- {old}` is not followed by a `+++` line"))?;
        let path = new
            .split('\t')
            .next()
            .unwrap_or(new)
            .trim()
            .strip_prefix("b/")
            .ok_or_else(|| format!("`+++ {new}` does not name a path as `b/<path>`"))?
            .to_owned();
        let mut hunks = Vec::new();
        while let Some(header) = lines.next_if(|l| l.starts_with("@@ ")) {
            let (old_start, old_count, new_count) = hunk_ranges(header)
                .ok_or_else(|| format!("{path}: cannot read the hunk header `{header}`"))?;
            let (mut old_left, mut new_left) = (old_count, new_count);
            let mut body = Vec::new();
            while old_left > 0 || new_left > 0 {
                let line = lines
                    .next()
                    .ok_or_else(|| format!("{path}: the hunk `{header}` ends early"))?;
                // Some tools drop the space of an empty context line.
                let (marker, content) = match line.chars().next() {
                    None => (' ', ""),
                    Some(marker) => (marker, &line[1..]),
                };
                match marker {
                    ' ' if old_left > 0 && new_left > 0 => {
                        old_left -= 1;
                        new_left -= 1;
                    }
                    '-' if old_left > 0 => old_left -= 1,
                    '+' if new_left > 0 => new_left -= 1,
                    _ => return Err(format!("{path}: unexpected line in `{header}`: `{line}`")),
                }
                body.push((marker, content.to_owned()));
            }
            if lines.peek().is_some_and(|l| l.starts_with('\\')) {
                return Err(format!(
                    "{path}: `\\ No newline at end of file` is not supported"
                ));
            }
            hunks.push(Hunk {
                header: header.to_owned(),
                old_start,
                lines: body,
            });
        }
        if hunks.is_empty() {
            return Err(format!("{path}: the file header has no hunk"));
        }
        files.push(FilePatch { path, hunks });
    }
    if files.is_empty() {
        return Err("it changes no file".to_owned());
    }
    Ok(files)
}

/// Reads `@@ -start,count +start,count @@`; a missing count is 1.
fn hunk_ranges(header: &str) -> Option<(usize, usize, usize)> {
    let mut parts = header.strip_prefix("@@ ")?.split_whitespace();
    let range = |part: &str| -> Option<(usize, usize)> {
        match part.split_once(',') {
            Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
            None => Some((part.parse().ok()?, 1)),
        }
    };
    let (old_start, old_count) = range(parts.next()?.strip_prefix('-')?)?;
    let (_, new_count) = range(parts.next()?.strip_prefix('+')?)?;
    Some((old_start, old_count, new_count))
}

/// Applies the hunks of one file. Each hunk must match at the line its header
/// names, context included, byte for byte, except that a trailing `\r` (a
/// checkout with `core.autocrlf` on Windows) is ignored on both sides of the
/// comparison. Lines the patch adds are written with the file's own line
/// ending, CRLF or LF, not the patch's.
pub(crate) fn apply_file_patch(original: &str, patch: &FilePatch) -> Result<String, String> {
    let newline = if original.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let lines: Vec<&str> = original.split_inclusive('\n').collect();
    let mut out = String::with_capacity(original.len());
    let mut cursor = 0;
    for hunk in &patch.hunks {
        let has_old = hunk.lines.iter().any(|(m, _)| *m != '+');
        // A hunk that removes nothing and keeps no context inserts after
        // `old_start`; any other starts at it.
        let start = if has_old {
            hunk.old_start.saturating_sub(1)
        } else {
            hunk.old_start
        };
        if start < cursor || start > lines.len() {
            return Err(format!(
                "the hunk `{}` is out of order or range",
                hunk.header
            ));
        }
        for line in &lines[cursor..start] {
            out.push_str(line);
        }
        let mut at = start;
        for (marker, content) in &hunk.lines {
            if *marker == '+' {
                out.push_str(content);
                out.push_str(newline);
                continue;
            }
            let found = lines
                .get(at)
                .map(|l| l.strip_suffix('\n').unwrap_or(l))
                .map(|l| l.strip_suffix('\r').unwrap_or(l));
            if found != Some(content.as_str()) {
                let found = found.map_or_else(
                    || "past the end of the file".to_owned(),
                    |found| format!("`{found}`"),
                );
                return Err(format!(
                    "the hunk `{}` expects line {} to be `{content}`, but it is {found}",
                    hunk.header,
                    at + 1
                ));
            }
            if *marker == ' ' {
                out.push_str(lines[at]);
            }
            at += 1;
        }
        cursor = at;
    }
    for line in &lines[cursor..] {
        out.push_str(line);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_file_patch(diff: &str) -> FilePatch {
        let mut files = parse_patch(diff).expect("a valid single-file diff");
        assert_eq!(files.len(), 1, "expected exactly one file in the diff");
        assert_eq!(files[0].path, "f", "path read from the `+++ b/<path>` line");
        files.remove(0)
    }

    #[test]
    fn context_and_removed_lines_match_a_crlf_file_ignoring_the_carriage_return() {
        // A file checked out with core.autocrlf on Windows: CRLF line
        // endings, while the patch (as git diff always writes it) has none.
        let original = "one\r\ntwo\r\nthree\r\n";
        let diff = [
            "--- a/f",
            "+++ b/f",
            "@@ -1,3 +1,3 @@",
            " one",
            "-two",
            "+TWO",
            " three",
            "",
        ]
        .join("\n");
        let patched = apply_file_patch(original, &one_file_patch(&diff)).expect("patch applies");
        assert_eq!(patched, "one\r\nTWO\r\nthree\r\n");
    }

    #[test]
    fn added_lines_take_the_file_s_own_line_ending() {
        let diff = [
            "--- a/f",
            "+++ b/f",
            "@@ -1,2 +1,3 @@",
            " one",
            "+inserted",
            " two",
            "",
        ]
        .join("\n");

        let crlf = apply_file_patch("one\r\ntwo\r\n", &one_file_patch(&diff))
            .expect("patch applies to a CRLF file");
        assert_eq!(crlf, "one\r\ninserted\r\ntwo\r\n");

        let lf = apply_file_patch("one\ntwo\n", &one_file_patch(&diff))
            .expect("patch applies to an LF file");
        assert_eq!(lf, "one\ninserted\ntwo\n");
    }

    #[test]
    fn a_mismatch_beyond_the_trailing_carriage_return_still_fails() {
        // The removed line really differs (TWO, not two), \r aside.
        let original = "one\r\nTWO\r\nthree\r\n";
        let diff = [
            "--- a/f",
            "+++ b/f",
            "@@ -1,3 +1,3 @@",
            " one",
            "-two",
            "+TWO",
            " three",
            "",
        ]
        .join("\n");
        let err = apply_file_patch(original, &one_file_patch(&diff))
            .expect_err("the removed line does not match beyond the \\r");
        assert!(err.contains("expects line 2"), "{err}");
    }
}
