//! C standard output that is still pending when the model printer starts must
//! come out before what the printer's closure writes.
//!
//! Safe Rust cannot leave bytes in C stdio's buffer, so this file uses
//! `libc::puts` from `main`. Expected order from pyclingo 5.8.2 (clingo 5.8.2):
//! C stdio is fully buffered on a pipe, and `println!` writes to file
//! descriptor 1 directly, so without the `fflush(NULL)` before the closure the
//! `pending` line would come after `closure`. The flush after `print()`
//! (checked in `application_printer_child.rs`) does not help here: the
//! closure's first line is written before `print()` runs.

#![cfg(unix)]
#![allow(
    unsafe_code,
    reason = "writing to C stdio without flushing is reachable only through libc"
)]
#![allow(
    clippy::unwrap_used,
    clippy::print_stdout,
    reason = "the child reports through standard output"
)]

#[path = "common/child.rs"]
mod child;

use clingox::Part;
use clingox::application::Application;

const ENTRY: &str = "child_entry";

/// The child side: runs the case named by the environment, or does nothing.
#[test]
fn child_entry() {
    let Some(case) = child::child_case() else {
        return;
    };
    assert_eq!(case, "pending");
    // Ends libtest's unfinished "test child_entry ... " line.
    println!();
    println!("CHILD-BEGIN");
    let result = Application::new()
        .main(|ctl, _files| {
            // SAFETY: `puts` reads a valid NUL-terminated string.
            unsafe { libc::puts(c"pending".as_ptr()) };
            ctl.add_base("a.")?;
            ctl.ground(&[Part::base()])?;
            let _ = ctl.solve(&[])?;
            Ok(())
        })
        .print_model(|_model, printer| {
            println!("closure");
            printer.print()
        })
        .run(["--verbose=0", "0"]);
    println!("CHILD-END");
    println!("CHILD-RC {}", result.unwrap());
}

#[test]
fn s1_pending_c_output_precedes_the_closures_first_line() {
    let Some(outcome) = child::run_child(ENTRY, "pending") else {
        return;
    };
    let out = &outcome.stdout;
    let start = out.find("CHILD-BEGIN\n").expect("begin marker") + "CHILD-BEGIN\n".len();
    let end = out.find("CHILD-END\n").expect("end marker");
    let lines: Vec<&str> = out[start..end].lines().collect();
    assert_eq!(lines, ["pending", "closure", "a", "SATISFIABLE"]);
    assert!(out.contains("CHILD-RC 30"), "{out}");
}
