use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

use crate::util::{Result, output, root, run};

const EMSDK_REPO: &str = "https://github.com/emscripten-core/emsdk";

/// The emsdk version the WASM build is tested with (DESIGN 9).
pub(crate) fn emsdk_version() -> Result<String> {
    let text = std::fs::read_to_string(root().join("xtask/emsdk-version"))?;
    Ok(text.trim().to_owned())
}

/// The repository-local emsdk, kept out of git by `.gitignore`.
pub(crate) fn emsdk_dir() -> PathBuf {
    root().join(".tools/emsdk")
}

pub(crate) fn wasm() -> Result<()> {
    let version = emsdk_version()?;
    let dir = emsdk_dir();
    if !dir.join("emsdk").exists() {
        std::fs::create_dir_all(dir.parent().expect(".tools/emsdk has a parent"))?;
        // The emsdk repository tags each release, so the installer script itself
        // is pinned along with the SDK it installs.
        run(Command::new("git")
            .args(["clone", "--depth", "1", "--branch", &version, EMSDK_REPO])
            .arg(&dir))?;
    }
    run(Command::new(dir.join("emsdk"))
        .args(["install", &version])
        .current_dir(&dir))?;
    run(Command::new(dir.join("emsdk"))
        .args(["activate", &version])
        .current_dir(&dir))?;
    eprintln!("emsdk {version} is installed in {}", dir.display());
    Ok(())
}

/// Nightly Rust with Miri and the standard library source it interprets.
pub(crate) fn miri() -> Result<()> {
    run(Command::new("rustup").args([
        "toolchain",
        "install",
        "nightly",
        "--profile",
        "minimal",
        "--component",
        "miri,rust-src",
    ]))?;
    // Builds Miri's sysroot once, so the first `cargo xtask miri` does not.
    run(Command::new("rustup").args(["run", "nightly", "cargo", "miri", "setup"]))?;
    eprintln!("Miri is installed for the nightly toolchain");
    Ok(())
}

/// The JavaScript tooling that runs the WASM tests in browsers.
pub(crate) fn browser_dir() -> PathBuf {
    root().join("xtask/browser")
}

pub(crate) fn browser() -> Result<()> {
    let dir = browser_dir();
    run(Command::new("npm").arg("ci").current_dir(&dir))?;
    // Each Playwright release drives specific builds of the three engines, so
    // the Playwright from the lockfile installs them.
    run(Command::new(dir.join("node_modules/.bin/playwright"))
        .args(["install", "chromium", "firefox", "webkit"])
        .current_dir(&dir))?;
    eprintln!("Playwright browsers are installed for {}", dir.display());
    Ok(())
}

/// The environment `emsdk_env.sh` sets up, so emcc, emcmake and Node.js from the
/// pinned SDK come first on PATH.
pub(crate) fn emsdk_env() -> Result<HashMap<String, String>> {
    let dir = emsdk_dir();
    let script = dir.join("emsdk_env.sh");
    if !script.exists() {
        return Err("the pinned emsdk is not installed; run `cargo xtask setup wasm`".into());
    }
    let dump = output(
        Command::new("bash")
            .arg("-c")
            .arg("source \"$1\" >/dev/null 2>&1 && env -0")
            .arg("bash")
            .arg(&script)
            .current_dir(&dir),
    )?;
    Ok(dump
        .split('\0')
        .filter_map(|entry| entry.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect())
}
