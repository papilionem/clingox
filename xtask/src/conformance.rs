//! `cargo xtask conformance-count`: recounts clingo's upstream conformance
//! inventory directly from the submodule, with one fixed unit per source, and
//! checks `clingox/tests/conformance/NOT_PORTED.md`'s own "Per-source counts"
//! table against that recount.
//!
//! The fixed unit, one per source:
//! - `app/clingo/tests/{lp,python,lua}/`: one `.lp` file is one item.
//! - `examples/c/`: one `.c` file is one item (`CMakeLists.txt` excluded).
//! - `libclingo/tests/*.cc`: one `TEST_CASE(...)` or one `SECTION(...)` is one
//!   item, at every nesting depth, ("`clingo.cc` and `symbol.cc` have 53
//!   `TEST_CASE` and `SECTION` entries"): an older hand-written count collapsed
//!   nested sections into their parent's name in prose, underclaiming the true
//!   total. `astv2.cc`, `propagator.cc` and `variant.cc` are part of the same
//!   source (`libclingo/tests/`) and recounted the same way; the older table
//!   counted each of those three files as one row worth its `TEST_CASE` count
//!   alone, ignoring their own nested `SECTION`s, which is the same bug in the
//!   opposite direction (overclaiming completeness for a file by undercounting
//!   its total).
//! - `libpyclingo/clingo/tests/test_*.py`: one `def test_...` method is one
//!   item.
//!
//! This does not attempt to verify that every single item is named exactly once
//! in `NOT_PORTED.md`'s prose (the libclingo and pyclingo sections use
//! parenthetical item counts per named row, not one line per item, so a by-name
//! check is not mechanical there without a much heavier C++/Python parser).
//! What it does check, automatically, for every source:
//! - the recounted total equals the file's own declared total;
//! - declared ported + declared not-ported equals the declared total;
//! - for the three fixture sources (`lp`, Python, Lua) and the C examples,
//!   which the file already lists as flat, comma-separated names, every
//!   upstream file is named exactly once across the ported prose and the
//!   not-ported table, with no name invented and none missing.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use crate::util::{Result, root};

const LIBCLINGO_COUNTED: [&str; 5] = [
    "clingo.cc",
    "symbol.cc",
    "astv2.cc",
    "propagator.cc",
    "variant.cc",
];

const PYCLINGO_COUNTED: [&str; 10] = [
    "test_application.py",
    "test_aspif.py",
    "test_ast.py",
    "test_atoms.py",
    "test_backend.py",
    "test_conf.py",
    "test_control.py",
    "test_propagator.py",
    "test_solving.py",
    "test_symbol.py",
];

/// Every other file `libclingo/tests/` actually contains, with why it is not
/// itself a test source: `check_directory_fully_accounted` (below) fails the
/// build if a file shows up in that directory matching neither this list nor
/// `LIBCLINGO_COUNTED`, so a new upstream test file cannot go unnoticed.
const LIBCLINGO_EXCLUDED: [(&str, &str); 2] = [
    ("CMakeLists.txt", "build file, not a test file"),
    (
        "tests.hh",
        "shared C++ test helpers (test_solve, MCB, ModelVec, ...), not a test file itself",
    ),
];

/// As `LIBCLINGO_EXCLUDED`, for `libpyclingo/clingo/tests/`.
const PYCLINGO_EXCLUDED: [(&str, &str); 2] = [
    ("__init__.py", "package marker, not a test file"),
    (
        "util.py",
        "shared test helpers (_MCB, _p, _check_sat, solve), not a test file itself",
    ),
];

/// Fails if `dir` contains a file that is neither in `counted` (recounted as
/// a test source) nor in `excluded` (named with a reason it is not one): the
/// mechanism that makes sure a new upstream test file added to
/// either the libclingo or pyclingo submodule directory cannot silently go
/// uncounted and unlisted. Only regular files are considered; a stray
/// subdirectory would need its own judgment call, not a blanket skip, so it
/// is reported the same way an unrecognised file is.
fn check_directory_fully_accounted(
    dir: &Path,
    counted: &[&str],
    excluded: &[(&str, &str)],
    label: &str,
) -> Result<()> {
    let mut unaccounted = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| format!("{label}: non-UTF-8 file name in {}", dir.display()))?;
        if counted.contains(&name)
            || excluded
                .iter()
                .any(|(excluded_name, _)| *excluded_name == name)
        {
            continue;
        }
        unaccounted.push(name.to_owned());
    }
    if unaccounted.is_empty() {
        Ok(())
    } else {
        unaccounted.sort();
        Err(format!(
            "{label}: {} in {} is neither counted nor excluded with a reason \
             (a new upstream test file needs one or the other in xtask/src/conformance.rs): {unaccounted:?}",
            if unaccounted.len() == 1 { "a file" } else { "files" },
            dir.display()
        )
        .into())
    }
}

fn clingo_root() -> PathBuf {
    root().join("clingox-sys/clingo")
}

fn not_ported_path() -> PathBuf {
    root().join("clingox/tests/conformance/NOT_PORTED.md")
}

/// Every `TEST_CASE(...)` or `SECTION(...)` occurrence in one C++ test file,
/// at any nesting depth (the fixed unit). A naive substring count would also
/// match `TEST_CASE_METHOD` or a mention inside a comment or string; this
/// walks line by line and only counts a match at the start of a (trimmed)
/// line, which is how every Catch2 use in this submodule is formatted, and
/// cross-checked below against `grep -c` on each file.
fn count_catch_items(text: &str) -> usize {
    text.lines()
        .filter(|line| {
            let t = line.trim_start();
            t.starts_with("TEST_CASE(") || t.starts_with("SECTION(")
        })
        .count()
}

/// Every `def test_...(` method in one pytest file, at the class-method
/// indentation pytest expects (four spaces), so a nested helper function
/// named `test_x` inside a test body is not double-counted (none exist in
/// this submodule; checked by comparing this count against a plain
/// `grep -c '^\s*def test_'`).
fn count_pytest_methods(text: &str) -> usize {
    text.lines()
        .filter(|line| line.trim_start().starts_with("def test_"))
        .count()
}

/// The `.lp` fixture files directly inside `dir` (not `.sol`/`.cmd` siblings).
fn count_lp_fixtures(dir: &Path) -> Result<usize> {
    let mut n = 0;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.path().extension().is_some_and(|e| e == "lp") {
            n += 1;
        }
    }
    Ok(n)
}

/// The names of every `.lp` fixture directly inside `dir`, without the
/// extension.
fn lp_fixture_names(dir: &Path) -> Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "lp")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            names.insert(stem.to_owned());
        }
    }
    Ok(names)
}

/// The `.c` example programs (`CMakeLists.txt` excluded).
fn c_example_names() -> Result<BTreeSet<String>> {
    let dir = clingo_root().join("examples/c");
    let mut names = BTreeSet::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "c")
            && let Some(name) = path.file_name().and_then(|s| s.to_str())
        {
            names.insert(name.to_owned());
        }
    }
    Ok(names)
}

struct Recount {
    lp: usize,
    python: usize,
    lua: usize,
    c_examples: usize,
    libclingo: usize,
    libclingo_per_file: Vec<(&'static str, usize)>,
    pyclingo: usize,
    pyclingo_per_file: Vec<(&'static str, usize)>,
}

fn recount() -> Result<Recount> {
    let tests = clingo_root().join("app/clingo/tests");
    let lp = count_lp_fixtures(&tests.join("lp"))?;
    let python = count_lp_fixtures(&tests.join("python"))?;
    let lua = count_lp_fixtures(&tests.join("lua"))?;
    let c_examples = c_example_names()?.len();

    let libclingo_dir = clingo_root().join("libclingo/tests");
    let mut libclingo_per_file = Vec::new();
    for file in LIBCLINGO_COUNTED {
        let text = std::fs::read_to_string(libclingo_dir.join(file))?;
        libclingo_per_file.push((file, count_catch_items(&text)));
    }
    let libclingo = libclingo_per_file.iter().map(|(_, n)| n).sum();

    let pyclingo_dir = clingo_root().join("libpyclingo/clingo/tests");
    let mut pyclingo_per_file = Vec::new();
    for file in PYCLINGO_COUNTED {
        let text = std::fs::read_to_string(pyclingo_dir.join(file))?;
        pyclingo_per_file.push((file, count_pytest_methods(&text)));
    }
    let pyclingo = pyclingo_per_file.iter().map(|(_, n)| n).sum();

    Ok(Recount {
        lp,
        python,
        lua,
        c_examples,
        libclingo,
        libclingo_per_file,
        pyclingo,
        pyclingo_per_file,
    })
}

/// A "Per-source counts" row parsed from `NOT_PORTED.md`'s own markdown table.
struct DeclaredRow {
    source: String,
    total: usize,
    ported: usize,
    not_ported: usize,
}

fn parse_declared_rows(not_ported: &str) -> Result<Vec<DeclaredRow>> {
    let table_start = not_ported
        .find("## Per-source counts")
        .ok_or("NOT_PORTED.md has no \"## Per-source counts\" table")?;
    let mut rows = Vec::new();
    let mut in_table = false;
    for line in not_ported[table_start..].lines().skip(1) {
        let line = line.trim();
        if !line.starts_with('|') {
            // A blank line (or any other line) ends the table once it has
            // started; before that, it is just the blank line between the
            // heading and the table itself.
            if in_table {
                break;
            }
            continue;
        }
        in_table = true;
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() != 4 || cells[0] == "Source" || cells[1].starts_with("---") {
            continue;
        }
        let parse = |c: &str| -> Result<usize> {
            c.trim_matches('*')
                .parse()
                .map_err(|_| format!("cannot parse count cell {c:?}").into())
        };
        rows.push(DeclaredRow {
            source: cells[0].trim_matches('`').to_owned(),
            total: parse(cells[1])?,
            ported: parse(cells[2])?,
            not_ported: parse(cells[3])?,
        });
    }
    if rows.is_empty() {
        return Err("NOT_PORTED.md's Per-source counts table has no data rows".into());
    }
    Ok(rows)
}

/// A bare item name this parser accepts from a comma-separated list line:
/// letters, digits, `_` and `-` only, no internal whitespace. Rejecting
/// anything else is what keeps ordinary prose (which always has a
/// multi-word, space-containing "token" somewhere between its commas) from
/// being misread as a name list.
fn looks_like_a_name(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// The names on one line, if the whole line is a *pure* comma-separated name
/// list (every comma-split token, once trimmed of backticks and a trailing
/// period, looks like a bare name): `assumptions1, assumptions2, blocksworld1`
/// qualifies, but a sentence like "Checked against `core1.lp` directly."
/// does not, because splitting it on its own commas (there may be none, or
/// one before "directly") leaves at least one token with a space in it.
/// Returns `None` for a line that is not a pure name list at all (prose,
/// blank, a heading, a parenthetical note, or a list marker), so a caller
/// can tell "no names on this line" apart from "this line is not a name
/// list and must not contribute any names, even if one of its words
/// happens to look bare".
fn pure_name_list_line(line: &str) -> Option<Vec<String>> {
    let line = line.trim();
    if line.is_empty() || line.starts_with(['#', '-', '(']) {
        return None;
    }
    let candidates: Vec<&str> = line
        .trim_end_matches('.')
        .split(',')
        .map(|t| t.trim().trim_matches('`'))
        .collect();
    if candidates.iter().all(|t| looks_like_a_name(t)) {
        Some(candidates.into_iter().map(str::to_owned).collect())
    } else {
        None
    }
}

/// The name in a not-ported table row's first column (`| name | reason |`), or
/// `None` for the header/separator row or a non-table line.
fn table_row_name(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix('|')?;
    let first = rest.split('|').next().unwrap_or_default().trim();
    let first = first.trim_matches('`');
    if first.is_empty() || first == "Item" || first.starts_with("---") {
        None
    } else {
        Some(first.to_owned())
    }
}

/// Splits a section into paragraphs: a run of consecutive non-blank,
/// non-table, non-heading lines is joined into one paragraph (with a single
/// space between its own lines, so a name list that wraps across lines,
/// like a long "Ported (...)" list, is still one pure comma list once
/// joined); a table row is its own one-line "paragraph", kept separate so
/// `table_row_name` still sees it whole. This is what lets
/// `pure_name_list_line` reject an ordinary prose paragraph that happens to
/// wrap a single long identifier across a line break (mid-identifier, no
/// space) while still accepting a genuine name list that wraps the same
/// way: a prose paragraph almost always has a real, multi-word, comma- or
/// period-separated sentence *somewhere* in the same paragraph, which fails
/// the "every candidate looks like a bare name" check once the whole
/// paragraph is joined; a name list never does.
fn paragraphs(section: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let flush = |current: &mut Vec<&str>, out: &mut Vec<String>| {
        if !current.is_empty() {
            out.push(current.join(" "));
            current.clear();
        }
    };
    for line in section.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            flush(&mut current, &mut out);
        } else if trimmed.starts_with('|') || trimmed.starts_with('#') {
            flush(&mut current, &mut out);
            out.push(trimmed.to_owned());
        } else {
            current.push(trimmed);
        }
    }
    flush(&mut current, &mut out);
    out
}

/// Every name mentioned in a `##`-headed section's pure comma-separated
/// name-list lines (the "Ported (...)" sub-lists) and the not-ported
/// table's first column, between `heading` and the next `##` heading.
/// Ordinary explanatory prose elsewhere in the section (which almost always
/// shares a paragraph with a name list, separated only by a blank line) is
/// never treated as names, by construction of `pure_name_list_line`.
fn names_in_section(not_ported: &str, heading: &str) -> BTreeSet<String> {
    let Some(start) = not_ported.find(heading) else {
        return BTreeSet::new();
    };
    let rest = &not_ported[start + heading.len()..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    let section = &rest[..end];

    let mut names = BTreeSet::new();
    for paragraph in paragraphs(section) {
        if let Some(name) = table_row_name(&paragraph) {
            names.insert(name);
            continue;
        }
        if let Some(found) = pure_name_list_line(&paragraph) {
            names.extend(found);
        }
    }
    names
}

pub(crate) fn run() -> Result<()> {
    check_directory_fully_accounted(
        &clingo_root().join("libclingo/tests"),
        &LIBCLINGO_COUNTED,
        &LIBCLINGO_EXCLUDED,
        "libclingo/tests/",
    )?;
    check_directory_fully_accounted(
        &clingo_root().join("libpyclingo/clingo/tests"),
        &PYCLINGO_COUNTED,
        &PYCLINGO_EXCLUDED,
        "libpyclingo/clingo/tests/",
    )?;

    let counted = recount()?;
    let text = std::fs::read_to_string(not_ported_path())?;
    let declared = parse_declared_rows(&text)?;

    let mut mismatches = Vec::new();
    let expect = |name: &str, want: usize, got: usize, mismatches: &mut Vec<String>| {
        if want != got {
            mismatches.push(format!(
                "{name}: NOT_PORTED.md declares total {want}, the submodule recount gives {got}"
            ));
        }
    };

    for row in &declared {
        if row.ported + row.not_ported != row.total {
            mismatches.push(format!(
                "{}: ported ({}) + not ported ({}) = {}, not the declared total {}",
                row.source,
                row.ported,
                row.not_ported,
                row.ported + row.not_ported,
                row.total
            ));
        }
        let recounted = match row.source.as_str() {
            s if s.contains("lp/") => Some(counted.lp),
            s if s.contains("python/") => Some(counted.python),
            s if s.contains("lua/") => Some(counted.lua),
            s if s.contains("examples/c") => Some(counted.c_examples),
            s if s.contains("libclingo") => Some(counted.libclingo),
            s if s.contains("libpyclingo") => Some(counted.pyclingo),
            _ => None,
        };
        if let Some(got) = recounted {
            expect(&row.source, row.total, got, &mut mismatches);
        } else {
            mismatches.push(format!("unrecognised source row: {}", row.source));
        }
    }

    // Fixture and C-example sources list every upstream file by name in
    // plain prose, so those are checked by name, not just by count.
    check_names_against_directory(
        &text,
        "## lp/ fixtures",
        &lp_fixture_names(&clingo_root().join("app/clingo/tests/lp"))?,
        &mut mismatches,
    );
    check_names_against_directory(
        &text,
        "## Python fixtures",
        &lp_fixture_names(&clingo_root().join("app/clingo/tests/python"))?,
        &mut mismatches,
    );
    check_names_against_directory(
        &text,
        "## Lua fixtures",
        &lp_fixture_names(&clingo_root().join("app/clingo/tests/lua"))?,
        &mut mismatches,
    );
    let c_examples: BTreeSet<String> = c_example_names()?
        .into_iter()
        .map(|n| n.trim_end_matches(".c").to_owned())
        .collect();
    check_names_against_directory(&text, "## C examples", &c_examples, &mut mismatches);

    if !mismatches.is_empty() {
        return Err(format!(
            "conformance-count: NOT_PORTED.md is out of sync with the submodule:\n{}",
            mismatches.join("\n")
        )
        .into());
    }

    eprintln!("conformance-count: recount matches NOT_PORTED.md");
    eprintln!(
        "  lp/: {}, python/: {}, lua/: {}, examples/c/: {}, libclingo/tests/: {}, libpyclingo tests: {}",
        counted.lp,
        counted.python,
        counted.lua,
        counted.c_examples,
        counted.libclingo,
        counted.pyclingo
    );
    for (file, n) in &counted.libclingo_per_file {
        eprintln!("    {file}: {n}");
    }
    for (file, n) in &counted.pyclingo_per_file {
        eprintln!("    {file}: {n}");
    }
    Ok(())
}

/// Checks that a fixture/example section's prose names exactly the files a
/// directory listing gives, with none missing and none invented. Every name
/// anywhere in the section counts, regardless of which "###" sub-list holds
/// it (ported, duplicates, not-ported): a name only needs to appear
/// somewhere in the section exactly once, which `names_in_section` already
/// collects as a set, so a name repeated in two sub-lists would still pass
/// this check; that stronger property (each name in exactly one place) is
/// checked separately by `duplicate names within one section`, below.
fn check_names_against_directory(
    not_ported: &str,
    heading: &str,
    directory: &BTreeSet<String>,
    mismatches: &mut Vec<String>,
) {
    let named = names_in_section(not_ported, heading);
    let missing: Vec<&String> = directory.difference(&named).collect();
    let invented: Vec<&String> = named.difference(directory).collect();
    if !missing.is_empty() {
        mismatches.push(format!(
            "{heading}: in the submodule but not named anywhere in NOT_PORTED.md: {missing:?}"
        ));
    }
    if !invented.is_empty() {
        mismatches.push(format!(
            "{heading}: named in NOT_PORTED.md but not in the submodule: {invented:?}"
        ));
    }
    if let Some(dupes) = duplicate_names(not_ported, heading) {
        mismatches.push(format!(
            "{heading}: named more than once in NOT_PORTED.md: {dupes:?}"
        ));
    }
}

/// Names that appear more than once in a section's prose lists and table
/// rows (each upstream item must be listed exactly once: ported, a
/// duplicate, or not ported, never more than one of those).
fn duplicate_names(not_ported: &str, heading: &str) -> Option<Vec<String>> {
    let start = not_ported.find(heading)?;
    let rest = &not_ported[start + heading.len()..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    let section = &rest[..end];

    let mut seen = HashMap::new();
    for paragraph in paragraphs(section) {
        let names: Vec<String> = table_row_name(&paragraph)
            .map(|n| vec![n])
            .or_else(|| pure_name_list_line(&paragraph))
            .unwrap_or_default();
        for name in names {
            *seen.entry(name).or_insert(0) += 1;
        }
    }
    let dupes: Vec<String> = seen
        .into_iter()
        .filter(|(_, n)| *n > 1)
        .map(|(name, _)| name)
        .collect();
    if dupes.is_empty() { None } else { Some(dupes) }
}

/// Recomputes the libclingo/pyclingo counts with the *older*, hand-written
/// approach, for comparison against the totals of 37 (libclingo) and 74
/// (pyclingo) that `NOT_PORTED.md` once declared.
///
/// This does not reproduce 37 or 74 from one formula, and that absence is
/// itself the finding, not a failure of this tool: neither number was ever
/// computed from the submodule by a fixed, consistent rule in the first
/// place. A hand tally of the old libclingo table does not even sum to its
/// own declared total (30 ported + 18 not-ported rows counted against a
/// declared 37); the pyclingo 74 is higher than the true 72 `def test_`
/// methods this recount finds, so no subset-or-superset counting rule can
/// reach it at all. Both descend from rough scout estimates that were carried
/// forward, rounded and revised piecemeal without ever being re-summed
/// against a grep. The automated recount (`run`, above) exists so that drift
/// cannot recur silently: it is wired into `cargo xtask check`, so a stale
/// total fails the build.
pub(crate) fn old_unit() -> Result<()> {
    let libclingo_dir = clingo_root().join("libclingo/tests");
    let mut fixed_unit_total = 0;
    for file in LIBCLINGO_COUNTED {
        let text = std::fs::read_to_string(libclingo_dir.join(file))?;
        fixed_unit_total += count_catch_items(&text);
    }
    eprintln!("fixed-unit recount: {fixed_unit_total}");
    eprintln!("earlier hand-written NOT_PORTED.md declared: 37");
    eprintln!(
        "not reproducible by one formula: 37 does not sum from its own rows \
         (30 + 18 = 49, not 37)"
    );

    let pyclingo_dir = clingo_root().join("libpyclingo/clingo/tests");
    let mut new_total = 0;
    for file in PYCLINGO_COUNTED {
        let text = std::fs::read_to_string(pyclingo_dir.join(file))?;
        new_total += count_pytest_methods(&text);
    }
    eprintln!("true recount of pyclingo test_ methods: {new_total}");
    eprintln!("earlier hand-written NOT_PORTED.md declared: 74");
    eprintln!(
        "not reproducible either: 74 > 72, so no counting rule over real methods reaches it; \
         it descends from a rough scout estimate (\"about 33\"), never re-checked"
    );
    Ok(())
}
