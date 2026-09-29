//! The command-line options that end the process from inside `clingo_main`.
//!
//! clingo's application leaves through `exit`, `_exit` or `std::_Exit` for a
//! handful of option combinations, and Rust never sees the call return. Each is
//! recognised here, before clingo starts, from the same spelling rules clasp's
//! option parser uses: a long option is matched by an exact name or by an
//! unambiguous prefix, a value follows `=` or is the next argument, and values
//! compare without regard to case. The table below lists the options and where
//! each one leaves the process.
//!
//! Short options are read the way clasp's `handleShortOpt` does
//! (`libpotassco/src/program_options.cpp`): the letters of one argument are
//! taken in turn, a flag letter (`-v`) goes on to the next letter, a letter
//! whose value is optional (`-s`, `-h`, `-V`, `-q`) or required (`-o`, `-t`,
//! `-n`, ...) takes the rest of the argument as its value, or, if that is
//! empty, a required one takes the next argument, and a letter that is no alias
//! ends the scan. So `-mo text` is `-m -o text` when the application registered
//! a flag `m`. The aliases the application registers are known only once its
//! `register_options` has run, so the arguments are checked twice: before
//! clingo starts with clingo's own aliases, and again, from inside the register
//! callback and before clingo parses, with the application's added.
//!
//! The check is deliberately a little wider than clingo: an argument after
//! `--`, which clingo reads as a file, is checked like any other, and `--pre`
//! is refused under every `--mode`. It is never narrower.

use std::path::Path;

/// How a one-letter alias takes its value (clasp's `Value` properties).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ShortKind {
    /// A flag: the scan goes on with the next letter of the argument.
    Flag,
    /// A value that may be left out: the rest of the argument is its value,
    /// possibly empty, and the next argument is never taken.
    Optional,
    /// A value that is required: the rest of the argument, or if that is
    /// empty the next argument.
    Required,
}

/// The one-letter aliases clingo registers itself.
const BUILTIN_ALIASES: &[(char, ShortKind)] = &[
    ('v', ShortKind::Flag),
    ('s', ShortKind::Optional),
    ('h', ShortKind::Optional),
    ('V', ShortKind::Optional),
    ('q', ShortKind::Optional),
    ('t', ShortKind::Required),
    ('e', ShortKind::Required),
    ('n', ShortKind::Required),
    ('r', ShortKind::Required),
    ('d', ShortKind::Required),
    ('c', ShortKind::Required),
    ('W', ShortKind::Required),
    ('o', ShortKind::Required),
    ('f', ShortKind::Required),
];

/// The one-letter alias in an option specification as
/// `clingo_options_add` takes it (`"name,x"`, `"name,x,@2"`), read as clasp's
/// `OptionInitHelper` reads it: after the first comma, a single byte that is
/// followed by the end or by a comma is the alias, whatever it is (`@` and `,`
/// included: `"name,@"` and `"name,,"` register those). A longer item, as in
/// `"name,@2"`, is a description level and no alias.
pub(super) fn alias_of(specification: &str) -> Option<char> {
    let (_, rest) = specification.split_once(',')?;
    let bytes = rest.as_bytes();
    let first = *bytes.first()?;
    (first.is_ascii() && bytes.get(1).is_none_or(|&next| next == b',')).then_some(first as char)
}

/// Returns why `arguments` would end the process, or `None` if clingo lets
/// them through (possibly with a command-line error of its own, which is an
/// exit code, not an exit). Only clingo's own short aliases are known.
pub(super) fn ends_process(arguments: &[&str]) -> Option<String> {
    ends_process_with(arguments, &[])
}

/// Whether `arguments` select `--mode=clasp`, spelled as `ends_process`
/// reads a mode.
pub(super) fn selects_clasp_mode(arguments: &[&str]) -> bool {
    let mut index = 0;
    while index < arguments.len() {
        let argument = arguments[index];
        index += 1;
        if let Some(rest) = argument.strip_prefix("--") {
            let (name, attached) = match rest.split_once('=') {
                Some((name, value)) => (name, Some(value)),
                None => (rest, None),
            };
            if name == "mode"
                && required(attached, arguments, &mut index)
                    .is_some_and(|v| v.eq_ignore_ascii_case("clasp"))
            {
                return true;
            }
        }
    }
    false
}

/// [`ends_process`] with the one-letter aliases the application registered
/// added to clingo's own.
pub(super) fn ends_process_with(
    arguments: &[&str],
    aliases: &[(char, ShortKind)],
) -> Option<String> {
    let mut text = None;
    let mut output = None;
    let mut wrong_mode = None;

    let mut index = 0;
    while index < arguments.len() {
        let argument = arguments[index];
        index += 1;
        if let Some(rest) = argument.strip_prefix("--") {
            let (name, attached) = match rest.split_once('=') {
                Some((name, value)) => (name, Some(value)),
                None => (rest, None),
            };
            let bare = attached.is_none_or(str::is_empty);
            match name {
                // The value is optional, so the next argument is not one.
                "pre" => {
                    if attached.is_none_or(|v| is_one_of(v, &["", "aspif", "smodels"])) {
                        return Some(refusal(argument, "prints the program and ends the process"));
                    }
                }
                "text" | "tex" => {
                    if bare {
                        text = Some(argument);
                    }
                }
                "output" => {
                    let value = required(attached, arguments, &mut index);
                    if value.is_some_and(|v| is_one_of(v, OUTPUT_FORMATS)) {
                        output = Some(argument);
                    }
                }
                "mode" => {
                    let value = required(attached, arguments, &mut index);
                    if value.is_some_and(|v| is_one_of(v, &["clingo", "clasp"])) {
                        wrong_mode = Some(argument);
                    }
                }
                "lemma-out" => {
                    if let Some(file) = required(attached, arguments, &mut index)
                        && !can_be_written(file)
                    {
                        return Some(refusal(
                            argument,
                            "names a file clasp cannot open for writing, which ends the process",
                        ));
                    }
                }
                // clasp accepts every unambiguous prefix, from `--out-a`.
                _ if name.len() >= 5 && "out-atomf".starts_with(name) => {
                    if let Some(format) = required(attached, arguments, &mut index)
                        && !atom_format_is_valid(format)
                    {
                        return Some(refusal(
                            argument,
                            "is a format clasp rejects while it sets up, which ends the process",
                        ));
                    }
                }
                _ if bare && name.len() >= 3 && "print-portfolio".starts_with(name) => {
                    return Some(refusal(
                        argument,
                        "prints the portfolio and ends the process",
                    ));
                }
                _ => {}
            }
        } else if let Some(group) = argument.strip_prefix('-') {
            // One argument of short options: `-o text`, `-otext`, `-mo text`.
            for (at, letter) in group.char_indices() {
                let Some(kind) = kind_of(letter, aliases) else {
                    break;
                };
                if kind == ShortKind::Flag {
                    continue;
                }
                let tail = &group[at + letter.len_utf8()..];
                let value = if tail.is_empty() && kind == ShortKind::Required {
                    arguments.get(index).inspect(|_| index += 1).copied()
                } else {
                    Some(tail)
                };
                if letter == 'o' && value.is_some_and(|v| is_one_of(v, OUTPUT_FORMATS)) {
                    output = Some(argument);
                }
                break;
            }
        }
    }

    match (text, output, wrong_mode) {
        (Some(t), Some(o), _) => Some(format!(
            "the arguments {t:?} and {o:?} are mutually exclusive, and clingo ends the \
             process (exit code 128) when it sees them together"
        )),
        (Some(a), None, Some(mode)) | (None, Some(a), Some(mode)) => Some(format!(
            "the arguments {a:?} and {mode:?} are incompatible, and clingo ends the process \
             (exit code 128) when it sees them together: they need --mode=gringo"
        )),
        _ => None,
    }
}

fn kind_of(letter: char, aliases: &[(char, ShortKind)]) -> Option<ShortKind> {
    BUILTIN_ALIASES
        .iter()
        .chain(aliases)
        .find(|(alias, _)| *alias == letter)
        .map(|&(_, kind)| kind)
}

/// `-o` and `--output` accept these, and only these.
const OUTPUT_FORMATS: &[&str] = &["intermediate", "text", "reify", "smodels"];

fn refusal(argument: &str, why: &str) -> String {
    format!("the argument {argument:?} {why}")
}

fn is_one_of(value: &str, candidates: &[&str]) -> bool {
    candidates.iter().any(|c| value.eq_ignore_ascii_case(c))
}

/// The value of an option that needs one: after `=` if that is not empty,
/// else the next argument, which is consumed.
fn required<'a>(
    attached: Option<&'a str>,
    arguments: &[&'a str],
    index: &mut usize,
) -> Option<&'a str> {
    match attached {
        Some(value) if !value.is_empty() => Some(value),
        _ => {
            let next = arguments.get(*index).copied();
            if next.is_some() {
                *index += 1;
            }
            next
        }
    }
}

/// Whether `fopen(file, "w")` would succeed, as `--lemma-out` needs, decided
/// without opening or creating anything (opening a named pipe would take its
/// reader, and creating would replace a dangling symlink by a file): a
/// directory fails; any other existing path is accepted; a path that does not
/// exist, or a dangling symlink (followed to the path clasp would create),
/// needs a parent directory that exists and that this process may write.
fn can_be_written(file: &str) -> bool {
    if file == "-" || file == "stdout" {
        return true;
    }
    let mut path = std::path::PathBuf::from(file);
    // A chain of dangling links is followed a few steps, as the system would.
    for _ in 0..40 {
        if let Ok(metadata) = std::fs::metadata(&path) {
            return !metadata.is_dir() && may_access(&path, false);
        }
        match std::fs::symlink_metadata(&path) {
            Ok(link) if link.file_type().is_symlink() => {
                let Ok(target) = std::fs::read_link(&path) else {
                    return false;
                };
                path = match path.parent() {
                    Some(parent) => parent.join(target),
                    None => target,
                };
            }
            // Missing, or unreadable for a reason that will fail the open too.
            Ok(_) => return true,
            Err(_) => break,
        }
    }
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    std::fs::metadata(parent).is_ok_and(|m| m.is_dir()) && may_access(parent, true)
}

/// Whether this process may write `path` (and, for a directory, enter it), by
/// the system's own check, which knows the real user, root and ACLs. The
/// answer can change before clasp opens the file; that race is left open.
#[cfg(unix)]
fn may_access(path: &Path, directory: bool) -> bool {
    use std::os::unix::ffi::OsStrExt;

    let Ok(c_path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let mode = if directory {
        libc::W_OK | libc::X_OK
    } else {
        libc::W_OK
    };
    // SAFETY: `c_path` is a NUL-terminated string that outlives the call, and
    // `access` only reads it.
    unsafe { libc::access(c_path.as_ptr(), mode) == 0 }
}

/// Without `access`, the mode bits of the file's own user are all there is to read.
#[cfg(not(unix))]
fn may_access(path: &Path, directory: bool) -> bool {
    std::fs::metadata(path).is_ok_and(|m| !directory || !m.permissions().readonly())
}

/// Whether clasp's `TextOutput` accepts `format` for `--out-atomf`: no
/// newline, `%` only as `%%` or one of `%s`, `%d`, `%0`, and a format for the
/// variable names that starts with `-`.
fn atom_format_is_valid(format: &str) -> bool {
    let mut specifier = None;
    let mut chars = format.chars();
    while let Some(c) = chars.next() {
        match c {
            '\n' => return false,
            '%' => match chars.next() {
                Some('%') => {}
                Some(kind @ ('s' | 'd' | '0')) if specifier.is_none() => specifier = Some(kind),
                _ => return false,
            },
            _ => {}
        }
    }
    // Only `%s` leaves the variable format at clasp's own `-%d`.
    format.is_empty() || matches!(specifier, Some('s' | '0')) || format.starts_with('-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(arguments: &[&str]) -> bool {
        ends_process(arguments).is_some()
    }

    #[test]
    fn pre_is_refused_bare_or_with_a_format() {
        for line in [
            &["--pre"][..],
            &["--pre=aspif"],
            &["--pre=SMODELS"],
            &["--pre="],
            &["a.lp", "--pre", "aspif"],
            &["--", "--pre"],
        ] {
            assert!(refused(line), "{line:?}");
        }
        for line in [
            &["--pre=text"][..],
            &["--pre=asp"],
            &["--pr"],
            &["--no-pre"],
            &["-pre"],
            &["--pres=1"],
        ] {
            assert!(!refused(line), "{line:?}");
        }
    }

    #[test]
    fn print_portfolio_is_refused_from_three_letters() {
        for end in 5..="--print-portfolio".len() {
            assert!(refused(&[&"--print-portfolio"[..end]]), "{end}");
        }
        for line in [
            &["--pr"][..],
            &["--p"],
            &["--print-portfolio=1"],
            &["--print-x"],
        ] {
            assert!(!refused(line), "{line:?}");
        }
    }

    #[test]
    fn text_and_output_together_are_refused() {
        assert!(refused(&["--text", "--output=text"]));
        assert!(refused(&["--tex", "--output", "REIFY"]));
        assert!(refused(&["-o", "smodels", "--text"]));
        assert!(refused(&["-otext", "--text"]));
        assert!(!refused(&["--text"]));
        assert!(!refused(&["--output=text"]));
        assert!(!refused(&["--text", "--output=aspif"]));
        assert!(!refused(&["--text", "--output"]));
        assert!(!refused(&["--te", "--output=text"]));
        assert!(!refused(&["--text=1", "-o"]));
    }

    #[test]
    fn text_or_output_with_another_mode_is_refused() {
        for mode in ["clingo", "clasp", "CLASP"] {
            assert!(refused(&["--text", &format!("--mode={mode}")]));
            assert!(refused(&["--mode", mode, "-o", "text"]));
        }
        assert!(!refused(&["--text", "--mode=gringo"]));
        assert!(!refused(&["--mode=clasp"]));
        assert!(!refused(&["--mode=clasp", "--output=aspif"]));
    }

    #[test]
    fn short_options_are_read_as_a_group() {
        let mine = [('m', ShortKind::Flag), ('k', ShortKind::Required)];
        let refused_with = |line: &[&str]| ends_process_with(line, &mine).is_some();
        // A registered flag lets `-o` follow in the same argument.
        assert!(refused_with(&["-mo", "text", "--mode=clingo"]));
        assert!(refused_with(&["-motext", "--text"]));
        assert!(refused_with(&["-mmo", "text", "--text"]));
        assert!(refused_with(&["-vo", "text", "--text"]));
        // Without the alias the scan stops at `m`, as clasp's does.
        assert!(!refused(&["-mo", "text", "--mode=clingo"]));
        // A letter with a value takes the rest of the argument.
        assert!(!refused_with(&["-kotext", "--text"]));
        assert!(!refused_with(&["-ko", "text", "--text"]));
        assert!(!refused_with(&["-nmotext", "--text"]));
        // A required value is the next argument, whatever it looks like.
        assert!(!refused_with(&["-n", "-o", "text", "--text"]));
        assert!(!refused_with(&["-k", "-o", "--text"]));
        // An optional value never takes the next argument.
        assert!(refused_with(&["-s", "-o", "text", "--text"]));
        assert!(!refused_with(&["-sotext", "--text"]));
    }

    #[test]
    fn aliases_are_read_from_the_option_specification() {
        assert_eq!(alias_of("mine,m"), Some('m'));
        assert_eq!(alias_of("mine,m,@2"), Some('m'));
        assert_eq!(alias_of("mine"), None);
        assert_eq!(alias_of("mine,@2"), None);
        assert_eq!(alias_of("mine,@"), Some('@'));
        assert_eq!(alias_of("mine,,"), Some(','));
        assert_eq!(alias_of("mine,,,@2"), Some(','));
        assert_eq!(alias_of("mine,,@2"), None);
        assert_eq!(alias_of("mine,mm"), None);
        assert_eq!(alias_of("mine,"), None);
    }

    #[test]
    fn clasp_mode_is_recognised_by_its_spellings() {
        assert!(selects_clasp_mode(&["--mode=clasp"]));
        assert!(selects_clasp_mode(&["a.lp", "--mode", "CLASP"]));
        assert!(!selects_clasp_mode(&["--mode=clingo"]));
        assert!(!selects_clasp_mode(&["--mod=clasp"]));
        assert!(!selects_clasp_mode(&["--mode"]));
    }

    #[test]
    fn atom_formats() {
        for good in ["", "%s", "a%sb", "-x%0", "%0", "-%d"] {
            assert!(atom_format_is_valid(good), "{good:?}");
        }
        for bad in [
            "x", "%", "0", "no", "1,2", "%s%d%%", "%%-%d", "%d%d", "a\nb", "%z", "/x/y",
        ] {
            assert!(!atom_format_is_valid(bad), "{bad:?}");
        }
    }

    #[test]
    fn out_atomf_is_matched_by_its_prefixes() {
        for name in [
            "--out-a=x",
            "--out-at=x",
            "--out-ato=x",
            "--out-atom=x",
            "--out-atomf=x",
        ] {
            assert!(refused(&[name]), "{name}");
        }
        assert!(refused(&["--out-a", "x"]));
        assert!(!refused(&["--out-=x", "--out=x", "--out-a=%s"]));
    }

    #[test]
    #[cfg_attr(miri, ignore = "touches the file system")]
    fn lemma_out_needs_a_file_that_opens() {
        let dir = std::env::temp_dir();
        assert!(may_access(&dir, true));
        assert!(!may_access(Path::new("/nonexistent/dir"), true));
        assert!(refused(&["--lemma-out=/nonexistent/dir/x"]));
        assert!(refused(&["--lemma-out", "/"]));
        assert!(!refused(&["--lemma-out=-"]));
        assert!(!refused(&["--lemma-out=stdout"]));
        assert!(!refused(&["--lemma-out"]));
    }
}
