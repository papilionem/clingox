//! The user documentation is complete and runs as tests: every Markdown file of
//! the guide with Rust code, and the README, is included in the crate's
//! doctests, and its code blocks follow the rules that make that work
//! (TESTING 8). `cargo xtask check` runs the doctests themselves.

#![forbid(unsafe_code)]
// These tests read the repository's files, which Android devices and browsers
// do not have. What they check does not depend on the target.
#![cfg(not(any(target_os = "android", target_family = "wasm")))]

use std::fs;
use std::path::{Path, PathBuf};

/// The chapters checked here, relative to `guide/src`.
const CHAPTERS: [&str; 8] = [
    "getting-started/installation.md",
    "getting-started/first-program.md",
    "tutorial/facts-and-rules.md",
    "tutorial/typed-results.md",
    "tutorial/solving-step-by-step.md",
    "tutorial/optimisation.md",
    "concepts/two-layers.md",
    "concepts/errors-and-panics.md",
];

/// The chapters that teach by example, relative to `guide/src`.
const CHAPTERS_WITH_EXAMPLES: [&str; 6] = [
    "getting-started/installation.md",
    "getting-started/first-program.md",
    "tutorial/facts-and-rules.md",
    "tutorial/typed-results.md",
    "tutorial/solving-step-by-step.md",
    "tutorial/optimisation.md",
];

/// rustdoc runs a code block as Rust when its info string starts with one of
/// these attributes, even without `rust`.
const RUSTDOC_ATTRIBUTES: [&str; 6] = [
    "ignore",
    "no_run",
    "compile_fail",
    "should_panic",
    "test_harness",
    "standalone_crate",
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the crate is inside the workspace")
        .to_path_buf()
}

fn files_with_extension(dir: &Path, extension: &str, found: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("the directory can be read").path();
        if path.is_dir() {
            files_with_extension(&path, extension, found);
        } else if path.extension().is_some_and(|e| e == extension) {
            found.push(path);
        }
    }
}

/// `README.md` and every Markdown file under `guide/src`.
fn markdown_files() -> Vec<PathBuf> {
    let mut files = vec![root().join("README.md")];
    files_with_extension(&root().join("guide/src"), "md", &mut files);
    files.sort();
    files
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn relative(path: &Path) -> String {
    path.strip_prefix(root())
        .unwrap_or(path)
        .display()
        .to_string()
}

/// A fenced code block: the line its fence opens on and its info string.
struct Block {
    line: usize,
    info: String,
}

impl Block {
    fn tokens(&self) -> Vec<&str> {
        self.info
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|t| !t.is_empty())
            .collect()
    }

    fn is_rust(&self) -> bool {
        let tokens = self.tokens();
        tokens.contains(&"rust")
            || tokens.first().is_some_and(|first| {
                RUSTDOC_ATTRIBUTES.contains(first) || first.starts_with("edition")
            })
    }

    fn is_runnable_rust(&self) -> bool {
        self.is_rust() && !self.tokens().contains(&"compile_fail")
    }
}

/// The fenced code blocks of a Markdown text (backtick or tilde fences).
fn code_blocks(text: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut open: Option<(char, usize)> = None;
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        let Some(fence_char) = trimmed.chars().next().filter(|c| *c == '`' || *c == '~') else {
            continue;
        };
        let run = trimmed.chars().take_while(|c| *c == fence_char).count();
        if run < 3 {
            continue;
        }
        let rest = trimmed[run..].trim();
        match open {
            None => {
                open = Some((fence_char, run));
                blocks.push(Block {
                    line: index + 1,
                    info: rest.to_owned(),
                });
            }
            Some((open_char, open_run)) => {
                if fence_char == open_char && run >= open_run && rest.is_empty() {
                    open = None;
                }
            }
        }
    }
    blocks
}

/// The paths of every `include_str!("…")` in the Rust files under
/// `clingox/src`, resolved against the including file's directory.
fn included_files() -> Vec<PathBuf> {
    let mut sources = Vec::new();
    files_with_extension(&root().join("clingox/src"), "rs", &mut sources);
    let mut included = Vec::new();
    for source in sources {
        let text = read(&source);
        let dir = source.parent().expect("a file has a directory");
        for piece in text.split("include_str!(").skip(1) {
            let piece = piece.trim_start();
            let Some(literal) = piece.strip_prefix('"') else {
                continue;
            };
            let Some(end) = literal.find('"') else {
                continue;
            };
            if let Ok(path) = dir.join(&literal[..end]).canonicalize() {
                included.push(path);
            }
        }
    }
    included
}

#[test]
fn every_markdown_file_with_rust_code_runs_as_a_doctest() {
    let included = included_files();
    let mut missing = Vec::new();
    for file in markdown_files() {
        let has_rust = code_blocks(&read(&file)).iter().any(Block::is_rust);
        let canonical = file.canonicalize().expect("the file exists");
        if has_rust && !included.contains(&canonical) {
            missing.push(relative(&file));
        }
    }
    assert!(
        missing.is_empty(),
        "these files have Rust code but no `include_str!` in a doctest module under \
         clingox/src: {missing:?}"
    );
}

#[test]
fn code_blocks_name_their_language_and_rust_blocks_run() {
    let mut problems = Vec::new();
    for file in markdown_files() {
        let text = read(&file);
        let name = relative(&file);
        for block in code_blocks(&text) {
            if block.info.is_empty() {
                problems.push(format!(
                    "{name}:{}: a code block without a language runs as Rust under rustdoc",
                    block.line
                ));
            }
            if block.is_rust()
                && block
                    .tokens()
                    .iter()
                    .any(|t| *t == "ignore" || *t == "no_run")
            {
                problems.push(format!(
                    "{name}:{}: `{}` does not run; every example must",
                    block.line, block.info
                ));
            }
        }
        if text.contains("{{#") {
            problems.push(format!(
                "{name}: mdBook include directives are not expanded by rustdoc"
            ));
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn the_new_chapters_exist_and_are_linked_from_the_summary() {
    let summary = read(&root().join("guide/src/SUMMARY.md"));
    for chapter in CHAPTERS {
        let path = root().join("guide/src").join(chapter);
        assert!(path.is_file(), "{} is missing", relative(&path));
        assert!(
            summary.contains(&format!("]({chapter})")),
            "SUMMARY.md does not link {chapter}"
        );
    }
}

#[test]
fn the_tutorials_and_the_readme_have_runnable_examples() {
    let mut files: Vec<PathBuf> = CHAPTERS_WITH_EXAMPLES
        .iter()
        .map(|chapter| root().join("guide/src").join(chapter))
        .collect();
    files.push(root().join("README.md"));
    for file in files {
        let blocks = code_blocks(&read(&file));
        assert!(
            blocks.iter().any(Block::is_runnable_rust),
            "{} has no runnable Rust example",
            relative(&file)
        );
    }
}

#[test]
fn the_readme_states_the_status_honestly() {
    let readme = read(&root().join("README.md"));
    assert!(
        !readme.contains("Nothing is bound"),
        "the README still says nothing is bound"
    );
    let lower = readme.to_lowercase();
    assert!(
        lower.contains("not published") || lower.contains("unpublished"),
        "the README must say that clingox is not published"
    );
    assert!(
        readme.contains("5.8.2"),
        "the README names the clingo version"
    );
}

#[test]
fn the_fence_reader_recognises_rust_blocks() {
    // A self-check of the helper the other tests rely on.
    let text = "```rust\nlet a = 1;\n```\n\n```text\nout\n```\n\n~~~toml\n[x]\n~~~\n\n\
                ```compile_fail\nx\n```\n\n````rust,ignore\n```\n````\n\n```\nbare\n```\n";
    let blocks = code_blocks(text);
    let infos: Vec<&str> = blocks.iter().map(|b| b.info.as_str()).collect();
    assert_eq!(
        infos,
        ["rust", "text", "toml", "compile_fail", "rust,ignore", ""]
    );
    let rust: Vec<bool> = blocks.iter().map(Block::is_rust).collect();
    assert_eq!(rust, [true, false, false, true, true, false]);
    assert!(!blocks[3].is_runnable_rust());
}
