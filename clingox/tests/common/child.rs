//! Helpers for the application tests that must run clingo in a child process:
//! `clingo_main` writes with C stdio, which libtest cannot capture, and some
//! paths end the process. A test file includes this module with `#[path =
//! "common/child.rs"] mod child;`.
//!
//! The child is the test binary itself, started with `--exact <entry>` and the
//! environment variable [`CASE_VAR`] naming the case; the entry test returns at
//! once when the variable is unset, so a normal run of the file skips it.

#![allow(dead_code, reason = "each test file uses a part of these helpers")]
#![allow(
    clippy::unwrap_used,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "test helpers fail loudly on unexpected errors"
)]

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

/// The environment variable that names the case a child runs.
pub(crate) const CASE_VAR: &str = "CLINGOX_CHILD_CASE";

/// How long a child may run before the parent kills it and fails.
const LIMIT: Duration = Duration::from_secs(30);

/// What a child did.
#[derive(Debug)]
pub(crate) struct Outcome {
    /// The exit code, or `None` when a signal killed the child.
    pub(crate) code: Option<i32>,
    /// The signal that killed the child (Unix only).
    pub(crate) signal: Option<i32>,
    /// Everything the child wrote to standard output, libtest's lines included.
    pub(crate) stdout: String,
    /// Everything the child wrote to standard error.
    pub(crate) stderr: String,
}

/// The case this process was started for, if it is a child.
pub(crate) fn child_case() -> Option<String> {
    std::env::var(CASE_VAR).ok()
}

/// Whether this host can start the test binary again: not WebAssembly, not
/// iOS (an app sandbox may not start processes, and a simulator run does not
/// show that), not Miri, and a probe run of `<exe> --list` works. A skipped case says so on
/// standard error.
pub(crate) fn can_spawn() -> bool {
    static CAN: OnceLock<bool> = OnceLock::new();
    *CAN.get_or_init(|| {
        if cfg!(any(target_family = "wasm", target_os = "ios", miri)) {
            return false;
        }
        let Ok(exe) = std::env::current_exe() else {
            return false;
        };
        Command::new(exe)
            .arg("--list")
            .stdin(Stdio::null())
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

/// Whether [`run_child_with`] can cap the child's address space: `ulimit -v`
/// is enforced (`RLIMIT_AS`) and accepted by `sh` on Linux and FreeBSD. Darwin
/// does not enforce `RLIMIT_AS` and its `sh` may reject the option, so the
/// `&&` would stop the child from running; OpenBSD's `ksh` has no `-v`;
/// Android is left out because its runtime reserves address space (see
/// `regression_range_overflow.rs`); a 32-bit process has no room for a 2 GiB
/// cap, which is its whole user space.
pub(crate) const CAN_CAP_ADDRESS_SPACE: bool = cfg!(all(
    any(target_os = "linux", target_os = "freebsd"),
    target_pointer_width = "64"
));

/// A file in the temporary directory, named for this process and `name`, with
/// `text` as its content.
pub(crate) fn fixture(name: &str, text: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("clingox_m5a_{}_{name}", std::process::id()));
    std::fs::write(&path, text).unwrap();
    path
}

/// `path` as text, for a command line.
pub(crate) fn arg(path: &std::path::Path) -> String {
    path.to_str().unwrap().to_owned()
}

fn drain(mut pipe: impl Read + Send + 'static) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

/// Runs the test binary again on the test `entry` with the case `case`,
/// standard input closed, and returns what it did. `None` (after a message on
/// standard error) when this host cannot start it.
///
/// # Panics
///
/// If the child runs longer than 30 seconds; it is killed first.
pub(crate) fn run_child(entry: &str, case: &str) -> Option<Outcome> {
    run_child_full(entry, case, LIMIT, None, "")
}

/// As [`run_child`], with `input` as the child's standard input (closed after
/// it is written).
pub(crate) fn run_child_with_stdin(entry: &str, case: &str, input: &str) -> Option<Outcome> {
    run_child_full(entry, case, LIMIT, None, input)
}

/// As [`run_child`], with its own time limit and an optional cap on the
/// child's address space in KiB (`ulimit -v`, applied by a shell that then
/// `exec`s the test binary), so a runaway allocation fails inside the child
/// instead of taking the machine down. The cap applies only where
/// [`CAN_CAP_ADDRESS_SPACE`] is true; elsewhere the child runs uncapped and
/// the time limit is the only guard.
///
/// # Panics
///
/// If the child runs longer than `limit`; it is killed first.
pub(crate) fn run_child_with(
    entry: &str,
    case: &str,
    limit: Duration,
    address_space_kib: Option<u64>,
) -> Option<Outcome> {
    run_child_full(entry, case, limit, address_space_kib, "")
}

/// The common body of the three helpers above.
fn run_child_full(
    entry: &str,
    case: &str,
    limit: Duration,
    address_space_kib: Option<u64>,
    input: &str,
) -> Option<Outcome> {
    if !can_spawn() {
        eprintln!("SKIPPED: {case}: this host cannot start the test binary again");
        return None;
    }
    let exe = std::env::current_exe().unwrap();
    let mut command = match address_space_kib {
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        Some(kib) => {
            let mut sh = Command::new("sh");
            sh.arg("-c")
                .arg(format!("ulimit -v {kib} && exec \"$0\" \"$@\""))
                .arg(exe);
            sh
        }
        _ => Command::new(exe),
    };
    let mut child = command
        .args(["--exact", entry, "--nocapture", "--test-threads=1"])
        .env(CASE_VAR, case)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        use std::io::Write as _;
        let mut stdin = child.stdin.take().unwrap();
        // a child that exits early closes the pipe; that is not our failure
        let _ = stdin.write_all(input.as_bytes());
    }
    let out = drain(child.stdout.take().unwrap());
    let err = drain(child.stderr.take().unwrap());
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            panic!("child {case} ran longer than {limit:?}");
        }
        thread::sleep(Duration::from_millis(20));
    };
    #[cfg(unix)]
    let signal = std::os::unix::process::ExitStatusExt::signal(&status);
    #[cfg(not(unix))]
    let signal = None;
    Some(Outcome {
        code: status.code(),
        signal,
        stdout: lines_as_written(out.join().unwrap()),
        stderr: lines_as_written(err.join().unwrap()),
    })
}

/// The text with `\n` line ends. A Windows child writes `\r\n` through C's text
/// mode, which is not what the tests compare.
fn lines_as_written(text: String) -> String {
    if cfg!(windows) {
        text.replace("\r\n", "\n")
    } else {
        text
    }
}
