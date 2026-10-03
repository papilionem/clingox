use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::setup::{browser_dir, emsdk_env};
use crate::util::{Result, cargo, output, root, run};

/// Crates whose tests run on device targets. xtask and systest are host tools:
/// systest compiles C against the header, which needs the host toolchain.
const TARGET_CRATES: [&str; 4] = ["-p", "clingox-sys", "-p", "clingox"];

const ANDROID_TARGET: &str = "x86_64-linux-android";
const ANDROID_ABI: &str = "x86_64";
/// Lowest API level the tests are built for; any `x86_64` emulator image runs it.
const ANDROID_API: &str = "24";
const DEVICE_DIR: &str = "/data/local/tmp/clingox-tests";

const WASM_TARGET: &str = "wasm32-unknown-emscripten";

pub(crate) fn linux() -> Result<()> {
    run(cargo().args(["test", "--workspace"]))
}

/// The file naming the Rust release the trybuild snapshots are made with.
const COMPILE_FAIL_TOOLCHAIN: &str = "xtask/compile-fail-toolchain";

/// The environment variable that makes the trybuild tests of `clingox` skip
/// themselves (see `clingox/tests/compile_fail.rs`).
const COMPILE_FAIL_SKIP: &str = "CLINGOX_SKIP_COMPILE_FAIL";

/// The trybuild tests, on the toolchain their snapshots were made with: the
/// release in [`COMPILE_FAIL_TOOLCHAIN`] with the `rust-src` component. Any
/// other setup fails here with the reason, rather than with a page of
/// snapshot differences.
pub(crate) fn compile_fail() -> Result<()> {
    let pinned = std::fs::read_to_string(root().join(COMPILE_FAIL_TOOLCHAIN))?;
    let pinned = pinned
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .ok_or_else(|| format!("{COMPILE_FAIL_TOOLCHAIN} names no version"))?;
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let version = output(Command::new(&rustc).arg("--version"))?;
    let release = version.split_whitespace().nth(1).unwrap_or_default();
    if release != pinned {
        return Err(format!(
            "the trybuild snapshots are made with rustc {pinned}, but this is `{}`; \
             use `rustup run {pinned} cargo xtask test compile-fail`, or set \
             {COMPILE_FAIL_SKIP}=1 to leave the tests out",
            version.trim()
        )
        .into());
    }
    let sysroot = output(Command::new(&rustc).args(["--print", "sysroot"]))?;
    if !Path::new(sysroot.trim())
        .join("lib/rustlib/src/rust/library")
        .is_dir()
    {
        return Err(format!(
            "the trybuild snapshots quote the standard library, which rustc does only with \
             the rust-src component: `rustup component add rust-src --toolchain {pinned}`"
        )
        .into());
    }
    run(cargo()
        .args([
            "test",
            "--locked",
            "-p",
            "clingox",
            "--test",
            "compile_fail",
        ])
        .env_remove(COMPILE_FAIL_SKIP))
}

/// Runs the Linux, Android and WASM suites at the same time: each as its own
/// invocation of `cargo xtask test <suite>`, so `test all` runs exactly the
/// commands `test linux`/`test android`/`test wasm` run on their own (same
/// tests, same flags, same failures). This is where the parallelism pays
/// off, since the Android run is mostly waiting on the device and the WASM
/// run never touches the host target that `test linux` builds for.
///
/// `sanitize` and `miri` stay their own, exclusive commands (TESTING,
/// "Build speed"): both instrument the whole dependency tree in their own
/// way, so running either alongside another heavy build would fight it for
/// memory rather than overlap productively.
///
/// Android and WASM each get their own `CARGO_TARGET_DIR` under
/// `target/parallel-test-all`, because Cargo takes a single lock over a
/// whole target directory for the duration of a build; without separate
/// directories the three suites would just serialise on that lock instead of
/// overlapping. `test linux` keeps the default target directory. The C++
/// side of each build stays cheap even so, because ccache (`clingox-sys`'s
/// `build.rs`) shares object files across target directories.
///
/// Output is captured per suite and printed as each suite finishes, under a
/// header naming it, so the three runs' output does not interleave.
pub(crate) fn all() -> Result<()> {
    let exe =
        std::env::current_exe().map_err(|e| format!("cannot find xtask's own executable: {e}"))?;
    let parallel_dir = root().join("target/parallel-test-all");

    let suites: [(&str, &[&str], Option<PathBuf>); 3] = [
        ("linux", &["test", "linux"], None),
        (
            "android",
            &["test", "android"],
            Some(parallel_dir.join("android")),
        ),
        ("wasm", &["test", "wasm"], Some(parallel_dir.join("wasm"))),
    ];

    let mut children = Vec::new();
    for (name, args, target_dir) in &suites {
        let mut cmd = Command::new(&exe);
        cmd.current_dir(root())
            .args(*args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = target_dir {
            cmd.env("CARGO_TARGET_DIR", dir);
        }
        eprintln!(
            "$ cargo xtask {} (background{})",
            args.join(" "),
            target_dir
                .as_ref()
                .map(|d| format!(", target dir {}", d.display()))
                .unwrap_or_default()
        );
        let child = cmd
            .spawn()
            .map_err(|e| format!("cannot start `cargo xtask {}`: {e}", args.join(" ")))?;
        children.push((*name, child));
    }

    let mut failed = Vec::new();
    for (name, child) in children {
        let out = child
            .wait_with_output()
            .map_err(|e| format!("{name}: cannot wait for it: {e}"))?;
        eprintln!("\n===== {name} =====");
        std::io::stdout().write_all(&out.stdout).ok();
        std::io::stderr().write_all(&out.stderr).ok();
        if !out.status.success() {
            failed.push(name);
        }
    }
    if failed.is_empty() {
        eprintln!("\ntest all: linux, android and wasm all passed");
        Ok(())
    } else {
        Err(format!("test all: failed: {}", failed.join(", ")).into())
    }
}

/// The unit tests of `clingox` under Miri (RULES 2, TESTING 7).
///
/// Miri cannot call into C, so it runs the tests of the `unsafe` primitives
/// and trampolines, which use fake C data; the tests that call clingo are
/// marked `cfg_attr(miri, ignore)`.
pub(crate) fn miri() -> Result<()> {
    let mut cmd = Command::new("rustup");
    cmd.current_dir(crate::util::root()).args([
        "run", "nightly", "cargo", "miri", "test", "-p", "clingox", "--lib",
    ]);
    crate::util::use_ccache_if_found(&mut cmd);
    run(&mut cmd).map_err(|e| format!("{e}; `cargo xtask setup miri` installs Miri").into())
}

pub(crate) fn android() -> Result<()> {
    let state = output(Command::new("adb").arg("get-state")).map_err(|e| {
        format!("no Android device is connected ({e}); start the x86_64 emulator first")
    })?;
    if state.trim() != "device" {
        return Err(format!("adb reports the device as {:?}", state.trim()).into());
    }
    let abi = output(Command::new("adb").args(["shell", "getprop", "ro.product.cpu.abi"]))?;
    if abi.trim() != ANDROID_ABI {
        return Err(format!("the connected device is {}, not {ANDROID_ABI}", abi.trim()).into());
    }

    let ndk = ndk_dir()?;
    eprintln!("using the NDK at {}", ndk.display());
    // cargo-ndk prints none of cargo's JSON messages, so the test executables
    // could not be found through it. The NDK's own clang wrappers carry the
    // target and API level, which is all cargo-ndk sets up for this target.
    let bin = ndk.join("toolchains/llvm/prebuilt/linux-x86_64/bin");
    let clang = bin.join(format!("{ANDROID_TARGET}{ANDROID_API}-clang"));
    let clangxx = bin.join(format!("{ANDROID_TARGET}{ANDROID_API}-clang++"));
    if !clang.exists() {
        return Err(format!("the NDK has no {}", clang.display()).into());
    }
    let target_env = ANDROID_TARGET.replace('-', "_");
    let json = output(
        cargo()
            .env("ANDROID_NDK_HOME", &ndk)
            .env("ANDROID_PLATFORM", ANDROID_API)
            .env(
                format!("CARGO_TARGET_{}_LINKER", target_env.to_uppercase()),
                &clang,
            )
            .env(format!("CC_{target_env}"), &clang)
            .env(format!("CXX_{target_env}"), &clangxx)
            .env(format!("AR_{target_env}"), bin.join("llvm-ar"))
            .args(["test", "--no-run", "--target", ANDROID_TARGET])
            .args(["--message-format=json-render-diagnostics"])
            .args(TARGET_CRATES)
            .stdout(Stdio::piped()),
    )?;
    let binaries = test_executables(&json)?;
    if binaries.is_empty() {
        return Err("cargo built no Android test executables".into());
    }

    let cxx_shared = ndk
        .join("toolchains/llvm/prebuilt/linux-x86_64/sysroot/usr/lib")
        .join(ANDROID_TARGET)
        .join("libc++_shared.so");
    run(Command::new("adb").args(["shell", "rm", "-rf", DEVICE_DIR]))?;
    run(Command::new("adb").args(["shell", "mkdir", "-p", DEVICE_DIR]))?;
    run(Command::new("adb")
        .arg("push")
        .arg(&cxx_shared)
        .arg(DEVICE_DIR))?;

    let mut failed = Vec::new();
    for binary in &binaries {
        let name = binary
            .file_name()
            .expect("executables have file names")
            .to_string_lossy();
        run(Command::new("adb")
            .arg("push")
            .arg(binary)
            .arg(format!("{DEVICE_DIR}/{name}")))?;
        // `adb shell` has not always forwarded the exit status, so the script
        // prints it and the marker is what decides.
        let script = format!(
            "cd {DEVICE_DIR} && chmod +x ./{name} && LD_LIBRARY_PATH={DEVICE_DIR} ./{name}; echo xtask-exit=$?"
        );
        eprintln!("$ adb shell {script}");
        let out = Command::new("adb").args(["shell", &script]).output()?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        print!("{stdout}");
        eprint!("{}", String::from_utf8_lossy(&out.stderr));
        // A debug test executable is about 70 MB, and an emulator's /data is
        // often only a few GB, so each one is removed as soon as it has run.
        run(Command::new("adb").args(["shell", "rm", "-f", &format!("{DEVICE_DIR}/{name}")]))?;
        if !stdout.lines().any(|l| l.trim() == "xtask-exit=0") {
            failed.push(name.into_owned());
        }
    }
    run(Command::new("adb").args(["shell", "rm", "-rf", DEVICE_DIR]))?;
    if failed.is_empty() {
        eprintln!("android: {} test executables passed", binaries.len());
        Ok(())
    } else {
        Err(format!("android: failed: {}", failed.join(", ")).into())
    }
}

/// Engines `--browser` accepts, in the order `--browser all` runs them.
const BROWSERS: [&str; 3] = ["chromium", "firefox", "webkit"];
const BROWSER_TIMEOUT_SECS: u64 = 120;

pub(crate) fn wasm(options: &[&str]) -> Result<()> {
    let (browsers, timeout) = wasm_options(options)?;
    let mut env = emsdk_env()?;
    // Rust's emscripten target keeps emscripten's fixed 16 MB heap, which
    // larger programs exhaust (clingo reports bad_alloc). Applications set
    // this themselves; the guide's platform chapter says so.
    env.insert(
        "CARGO_TARGET_WASM32_UNKNOWN_EMSCRIPTEN_RUSTFLAGS".into(),
        "-C link-arg=-sALLOW_MEMORY_GROWTH=1".into(),
    );
    if browsers.is_empty() {
        let mut unsupported = 0;
        for profile in [None, Some("--release")] {
            let mut cmd = cargo();
            cmd.envs(&env)
                .env("CARGO_TARGET_WASM32_UNKNOWN_EMSCRIPTEN_RUNNER", "node")
                .args(["test", "--target", WASM_TARGET])
                .args(TARGET_CRATES)
                .args(profile);
            unsupported += run_counting(&mut cmd, UNSUPPORTED_SYSCALL)?;
        }
        // Emscripten's debug libc reports a syscall it only stubs, and the stub
        // returns made-up values. clingo's one such call, clasp's getrusage, is
        // patched out (U14), so a report means a new stubbed call to look at.
        if unsupported > 0 {
            return Err(format!(
                "wasm: {unsupported} lines report `{UNSUPPORTED_SYSCALL}`; a call into an \
                 Emscripten stub needs a patch or a note (UPSTREAM-ISSUES U14)"
            )
            .into());
        }
        return Ok(());
    }

    let dir = browser_dir();
    if !dir.join("node_modules/playwright").exists() {
        return Err(
            "the browser test tooling is not installed; run `cargo xtask setup browser`".into(),
        );
    }
    let mut executables = Vec::new();
    for profile in [None, Some("--release")] {
        let json = output(
            cargo()
                .envs(&env)
                .args(["test", "--no-run", "--target", WASM_TARGET])
                .args(["--message-format=json-render-diagnostics"])
                .args(TARGET_CRATES)
                .args(profile)
                .stdout(Stdio::piped()),
        )?;
        executables.extend(test_executables(&json)?);
    }
    if executables.is_empty() {
        return Err("cargo built no WASM test executables".into());
    }

    let result_file =
        std::env::temp_dir().join(format!("clingox-browser-{}.json", std::process::id()));
    let mut summary = Vec::new();
    let mut all_passed = true;
    for browser in browsers {
        // A result left over from the previous engine must not stand in for
        // this one if the runner dies before writing its own.
        let _ = std::fs::remove_file(&result_file);
        eprintln!("$ node xtask/browser/run.mjs --browser {browser} --timeout {timeout} ...");
        let status = Command::new("node")
            .arg(dir.join("run.mjs"))
            .args(["--browser", browser, "--timeout", &timeout.to_string()])
            .arg("--result")
            .arg(&result_file)
            .args(&executables)
            .status()
            .map_err(|e| format!("cannot start node: {e}"))?;
        let verdict = browser_verdict(&result_file, &executables, status.success());
        let _ = std::fs::remove_file(&result_file);
        let total = executables.len();
        match verdict {
            Ok(()) => summary.push(format!("{browser}: {total} test executables, all passed")),
            Err(err) => {
                all_passed = false;
                summary.push(format!(
                    "{browser}: {total} test executables, FAILED: {err}"
                ));
            }
        }
    }
    eprintln!();
    for line in &summary {
        eprintln!("{line}");
    }
    if all_passed {
        Ok(())
    } else {
        Err("the WASM tests failed in at least one browser".into())
    }
}

/// Parses `[--browser <engine>|all] [--timeout <seconds>]`. No `--browser`
/// means the Node.js run.
/// What Emscripten's debug libc prints when a program calls a syscall that it
/// only stubs (`system/lib/libc/emscripten_syscall_stubs.c`).
const UNSUPPORTED_SYSCALL: &str = "unsupported syscall";

/// As [`run`], and counts the lines of stdout and stderr that contain
/// `needle`. Both streams are passed through unchanged, as raw bytes.
fn run_counting(cmd: &mut Command, needle: &str) -> Result<usize> {
    fn copy(from: impl std::io::Read, mut to: impl Write, needle: &[u8]) -> std::io::Result<usize> {
        let mut from = std::io::BufReader::new(from);
        let mut line = Vec::new();
        let mut hits = 0;
        loop {
            line.clear();
            if std::io::BufRead::read_until(&mut from, b'\n', &mut line)? == 0 {
                return Ok(hits);
            }
            if line.windows(needle.len()).any(|w| w == needle) {
                hits += 1;
            }
            to.write_all(&line)?;
        }
    }
    eprintln!("$ {cmd:?}");
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start {cmd:?}: {e}"))?;
    let (stdout, stderr) = (child.stdout.take(), child.stderr.take());
    let (stdout, stderr) = (
        stdout.ok_or("no stdout pipe")?,
        stderr.ok_or("no stderr pipe")?,
    );
    let needle_out = needle.as_bytes().to_vec();
    let out = std::thread::spawn(move || copy(stdout, std::io::stdout(), &needle_out));
    let err_hits = copy(stderr, std::io::stderr(), needle.as_bytes())?;
    let out_hits = out.join().map_err(|_| "the stdout copy panicked")??;
    let status = child.wait()?;
    if !status.success() {
        return Err(format!("`{cmd:?}` failed with {status}").into());
    }
    Ok(out_hits + err_hits)
}

fn wasm_options(options: &[&str]) -> Result<(Vec<&'static str>, u64)> {
    let mut browsers = Vec::new();
    let mut timeout = None;
    let mut rest = options.iter();
    while let Some(option) = rest.next() {
        match *option {
            "--browser" => {
                let value = rest.next().ok_or("--browser needs an engine")?;
                browsers = match *value {
                    "all" => BROWSERS.to_vec(),
                    name => vec![*BROWSERS.iter().find(|b| **b == name).ok_or_else(|| {
                        format!("unknown browser {name:?}; use chromium, firefox, webkit or all")
                    })?],
                };
            }
            "--timeout" => {
                let value = rest.next().ok_or("--timeout needs a number of seconds")?;
                let secs: u64 = value
                    .parse()
                    .map_err(|_| format!("--timeout {value:?} is not a number of seconds"))?;
                if secs == 0 {
                    return Err("--timeout must be at least 1 second".into());
                }
                timeout = Some(secs);
            }
            other => return Err(format!("unknown option {other:?} for `test wasm`").into()),
        }
    }
    if timeout.is_some() && browsers.is_empty() {
        return Err("--timeout applies only to --browser runs".into());
    }
    Ok((browsers, timeout.unwrap_or(BROWSER_TIMEOUT_SECS)))
}

/// Checks the runner's result file and exit status against the executables it
/// was given. A pass needs both to agree and every executable to be listed as
/// passed, so a runner that exits 0 early, or skips one, still fails.
fn browser_verdict(result_file: &Path, executables: &[PathBuf], exited_ok: bool) -> Result<()> {
    let text = std::fs::read_to_string(result_file)
        .map_err(|_| "the runner exited without writing a result")?;
    let result: serde_json::Value = serde_json::from_str(&text)?;
    if result["launchError"].is_string() {
        return Err(
            "none ran, Playwright cannot launch this browser here (its message is above)".into(),
        );
    }
    let failed = result["failed"].as_array().map_or(0, Vec::len);
    if failed > 0 {
        return Err(format!("{failed} failed").into());
    }
    let passed: Vec<&str> = result["passed"]
        .as_array()
        .map(|list| list.iter().filter_map(|p| p.as_str()).collect())
        .unwrap_or_default();
    let every_one_passed = passed.len() == executables.len()
        && executables
            .iter()
            .all(|exe| passed.iter().any(|p| Path::new(p) == exe));
    if !exited_ok || !every_one_passed {
        return Err(format!(
            "the runner reported {} of {} as passed and exited {}",
            passed.len(),
            executables.len(),
            if exited_ok {
                "successfully"
            } else {
                "with an error"
            },
        )
        .into());
    }
    Ok(())
}

/// The test executables listed in cargo's JSON messages.
fn test_executables(json: &str) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    for line in json.lines().filter(|l| l.starts_with('{')) {
        let message: serde_json::Value = serde_json::from_str(line)?;
        let is_test = message["profile"]["test"].as_bool() == Some(true);
        if message["reason"] == "compiler-artifact"
            && is_test
            && let Some(exe) = message["executable"].as_str()
        {
            found.push(PathBuf::from(exe));
        }
    }
    Ok(found)
}

/// The NDK to use: the one named by the usual variables, otherwise the newest
/// one installed in the SDK.
fn ndk_dir() -> Result<PathBuf> {
    for var in ["ANDROID_NDK_HOME", "ANDROID_NDK_ROOT"] {
        if let Some(dir) = std::env::var_os(var) {
            return Ok(PathBuf::from(dir));
        }
    }
    let sdk = ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
        .iter()
        .find_map(std::env::var_os)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join("Android/Sdk")))
        .ok_or("cannot find the Android SDK; set ANDROID_HOME")?;
    let mut versions: Vec<(Vec<u64>, PathBuf)> = std::fs::read_dir(sdk.join("ndk"))?
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let version: Option<Vec<u64>> = name.split('.').map(|p| p.parse().ok()).collect();
            Some((version?, entry.path()))
        })
        .collect();
    versions.sort();
    versions
        .pop()
        .map(|(_, path)| path)
        .ok_or_else(|| format!("no NDK under {}", sdk.join("ndk").display()).into())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn sh(script: &str) -> Command {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", script]);
        cmd
    }

    #[test]
    fn run_counting_counts_matching_lines_on_both_streams() {
        let script = "echo 'warning: unsupported syscall: a'; echo fine; \
                      echo 'warning: unsupported syscall: b' >&2; printf 'no newline: unsupported syscall' >&2";
        assert_eq!(
            run_counting(&mut sh(script), UNSUPPORTED_SYSCALL).unwrap(),
            3
        );
    }

    #[test]
    fn run_counting_passes_bytes_that_are_not_utf8() {
        assert_eq!(
            run_counting(&mut sh(r"printf 'a\377b\n'"), UNSUPPORTED_SYSCALL).unwrap(),
            0
        );
    }

    #[test]
    fn run_counting_fails_on_a_failing_command() {
        assert!(
            run_counting(
                &mut sh("echo unsupported syscall; exit 3"),
                UNSUPPORTED_SYSCALL
            )
            .is_err()
        );
    }
}
