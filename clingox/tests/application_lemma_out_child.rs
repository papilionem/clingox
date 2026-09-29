//! `--lemma-out=<file>` through `Application::run`: what `run` may and may not
//! do with the path before clingo starts.
//!
//! clasp opens the file with `fopen(file, "w")` and ends the process when that
//! fails. `run` tells the two apart without touching the path: a FIFO with a
//! reader attached must still work (opening it for writing would block until a
//! reader appears, and would eat the reader's end), a dangling symlink whose
//! directory exists must stay a symlink and receive the lemmas (creating and
//! removing a probe file would replace it), and a directory is refused.
//! Expected behaviour is pyclingo 5.8.2's `clingo_main` (2026-09-29): the
//! pigeonhole program below is unsatisfiable (return code 20) and writes about
//! 5 kB of aspif lemmas starting with `asp 1 0 0`; a dangling symlink is
//! followed and its target created, the link staying a link; a directory or a
//! missing parent directory exits with 1.
//!
//! Every case runs in a child (`common/child.rs`); the case name carries the
//! paths, which the parent prepares. Unix only, and skipped where `mkfifo` is
//! not available.

#![cfg(unix)]
#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "test helpers fail loudly, and the child reports through standard output"
)]

#[path = "common/child.rs"]
mod child;

use std::io::Read;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use clingox::ErrorKind;
use clingox::application::Application;

const ENTRY: &str = "child_entry";
/// Six pigeons, five holes: unsatisfiable, with conflicts to learn from.
const PIGEONS: &str = "1 {p(P,H) : H=1..5} 1 :- P=1..6. :- p(P,H), p(Q,H), P<Q.";

fn mark(text: &str) {
    println!("\nCHILD-{text}");
}

fn report(result: clingox::Result<i32>) {
    match result {
        Ok(code) => mark(&format!("RC {code}")),
        Err(error) => mark(&format!("ERR {:?}", error.kind())),
    }
}

/// The child side: the case is `<kind>|<path>`.
#[test]
fn child_entry() {
    let Some(case) = child::child_case() else {
        return;
    };
    let (kind, path) = case.split_once('|').unwrap();
    let program = child::fixture("lemma_pigeons.lp", PIGEONS);
    let args = [
        child::arg(&program),
        format!("--lemma-out={path}"),
        "--outf=3".to_owned(),
    ];
    match kind {
        "fifo" => {
            let reader_path = path.to_owned();
            let reader = std::thread::spawn(move || {
                let mut text = String::new();
                std::fs::File::open(reader_path)
                    .unwrap()
                    .read_to_string(&mut text)
                    .unwrap();
                text
            });
            report(Application::new().run(args));
            let text = reader.join().unwrap();
            mark(&format!(
                "LEMMAS {} {}",
                text.len(),
                text.lines().next().unwrap_or_default()
            ));
        }
        "plain" | "symlink" | "dir" | "missing" => report(Application::new().run(args)),
        other => panic!("unknown child case {other}"),
    }
    mark("ALIVE");
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("clingox_lemma_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(kind: &str, path: &Path) -> Option<child::Outcome> {
    child::run_child(ENTRY, &format!("{kind}|{}", path.display()))
}

#[test]
fn a_writable_file_receives_the_lemmas() {
    let dir = scratch("plain");
    let file = dir.join("lemmas.txt");
    let Some(out) = run("plain", &file) else {
        return;
    };
    assert!(out.stdout.contains("CHILD-RC 20"), "{}", out.stdout);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.starts_with("asp 1 0 0"), "{text:.80}");
    assert!(text.len() > 1000, "{}", text.len());
}

#[test]
fn a_fifo_with_a_reader_gets_the_lemmas_and_the_run_does_not_hang() {
    let dir = scratch("fifo");
    let fifo = dir.join("lemmas.fifo");
    let made = Command::new("mkfifo")
        .arg(&fifo)
        .stdin(Stdio::null())
        .status();
    if !made.is_ok_and(|s| s.success()) {
        eprintln!("SKIPPED: mkfifo is not available");
        return;
    }
    let Some(out) = run("fifo", &fifo) else {
        return;
    };
    assert!(out.stdout.contains("CHILD-RC 20"), "{}", out.stdout);
    assert!(out.stdout.contains("CHILD-LEMMAS "), "{}", out.stdout);
    let line = out
        .stdout
        .lines()
        .find(|l| l.starts_with("CHILD-LEMMAS "))
        .unwrap();
    let mut parts = line.splitn(3, ' ');
    parts.next();
    let length: usize = parts.next().unwrap().parse().unwrap();
    assert!(length > 1000, "{line}");
    assert_eq!(parts.next(), Some("asp 1 0 0"), "{line}");
    assert!(out.stdout.contains("CHILD-ALIVE"), "{}", out.stdout);
    assert!(
        std::fs::symlink_metadata(&fifo)
            .unwrap()
            .file_type()
            .is_fifo(),
        "the fifo is still a fifo"
    );
}

#[test]
fn a_dangling_symlink_stays_a_symlink_and_its_target_is_written() {
    let dir = scratch("symlink");
    let target = dir.join("target.txt");
    let link = dir.join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let Some(out) = run("symlink", &link) else {
        return;
    };
    assert!(out.stdout.contains("CHILD-RC 20"), "{}", out.stdout);
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink(),
        "the link was replaced"
    );
    let text = std::fs::read_to_string(&target).expect("the target was created");
    assert!(text.starts_with("asp 1 0 0"), "{text:.80}");
}

#[test]
fn a_directory_is_refused() {
    let dir = scratch("dir");
    let Some(out) = run("dir", &dir) else {
        return;
    };
    assert!(
        out.stdout
            .contains(&format!("CHILD-ERR {:?}", ErrorKind::InvalidInput)),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("CHILD-ALIVE"), "{}", out.stdout);
}

#[test]
fn a_missing_parent_directory_is_refused_and_nothing_is_created() {
    let dir = scratch("missing");
    let file = dir.join("no").join("such").join("lemmas.txt");
    let Some(out) = run("missing", &file) else {
        return;
    };
    assert!(
        out.stdout
            .contains(&format!("CHILD-ERR {:?}", ErrorKind::InvalidInput)),
        "{}",
        out.stdout
    );
    assert!(!dir.join("no").exists());
}
