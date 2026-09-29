use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub(crate) type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The workspace root, which every path in xtask is relative to.
pub(crate) fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives one level below the workspace root")
        .to_path_buf()
}

pub(crate) fn cargo() -> Command {
    let mut cmd = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cmd.current_dir(root());
    use_ccache_if_found(&mut cmd);
    cmd
}

/// Sets `CLINGOX_SYS_CCACHE=auto` on `cmd` when `ccache` is on `PATH`, so
/// `clingox-sys`'s build script (`forward_compiler_launcher`) uses it for
/// `xtask`'s own builds (TESTING.md, "Build speed"). `clingox-sys` defaults
/// to no ccache, since it is a published crate and must not reach outside
/// the build unless asked; this is `xtask` doing the asking, but only when
/// the environment does not already say something explicit about it (`0` to
/// opt out, or an explicit path): an explicit `CLINGOX_SYS_CCACHE` always
/// wins over this default.
///
/// Called from [`cargo()`]; also called directly by callers that build cargo
/// commands of their own, such as `sanitize::sanitized_cargo` and
/// `test::miri`, which run `cargo` through `rustup` rather than through this
/// module's `cargo()`.
pub(crate) fn use_ccache_if_found(cmd: &mut Command) {
    if std::env::var_os("CLINGOX_SYS_CCACHE").is_none() && ccache_on_path() {
        cmd.env("CLINGOX_SYS_CCACHE", "auto");
    }
}

fn ccache_on_path() -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join("ccache").is_file()))
}

/// Runs `cmd` with inherited stdio and fails if it exits unsuccessfully.
pub(crate) fn run(cmd: &mut Command) -> Result<()> {
    eprintln!("$ {}", describe(cmd));
    let status = cmd
        .status()
        .map_err(|e| format!("cannot start {}: {e}", describe(cmd)))?;
    if !status.success() {
        return Err(format!("`{}` failed with {status}", describe(cmd)).into());
    }
    Ok(())
}

/// Runs `cmd` and returns its stdout; stderr is passed through.
pub(crate) fn output(cmd: &mut Command) -> Result<String> {
    let out = cmd
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("cannot start {}: {e}", describe(cmd)))?;
    if !out.status.success() {
        return Err(format!("`{}` failed with {}", describe(cmd), out.status).into());
    }
    Ok(String::from_utf8(out.stdout)?)
}

fn describe(cmd: &Command) -> String {
    let mut s = cmd.get_program().to_string_lossy().into_owned();
    for arg in cmd.get_args() {
        s.push(' ');
        s.push_str(&arg.to_string_lossy());
    }
    s
}

/// The workspace MSRV, read from the root manifest so it is stated only once.
pub(crate) fn msrv() -> Result<(u64, u64)> {
    let manifest = std::fs::read_to_string(root().join("Cargo.toml"))?;
    let line = manifest
        .lines()
        .find_map(|l| l.trim().strip_prefix("rust-version"))
        .ok_or("no rust-version in the workspace manifest")?;
    let value = line.trim_start_matches([' ', '=']).trim().trim_matches('"');
    let mut parts = value.split('.').map(str::parse::<u64>);
    match (parts.next(), parts.next()) {
        (Some(Ok(1)), Some(Ok(minor))) => Ok((
            minor,
            parts.next().and_then(std::result::Result::ok).unwrap_or(0),
        )),
        _ => Err(format!("cannot parse rust-version {value:?}").into()),
    }
}
