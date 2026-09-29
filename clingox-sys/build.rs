//! Builds clingo from the vendored submodule, or finds a system installation,
//! and emits the link lines for it (DESIGN 5.1, 5.2 and 9).
//!
//! A vendored build compiles a copy of the submodule with the patches in
//! `patches/` applied (RULES 8); the submodule itself is never written.

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

// A separate module, not inline, so `clingox-sys/tests/patches.rs` can
// include the same source and unit-test the applier directly.
#[path = "build/patch_applier.rs"]
mod patch_applier;
use patch_applier::{apply_file_patch, parse_patch};

/// Static libraries in link order: libgringo uses libreify, and single-pass
/// linkers need every user before the library it uses (DESIGN 5.2).
const LIBS: [&str; 5] = ["clingo", "gringo", "reify", "clasp", "potassco"];

/// System libraries older than 5.8.1 are refused: concurrent symbol creation
/// became safe only in 5.8.1 (DESIGN 5.1).
const MIN_VERSION: (u32, u32, u32) = (5, 8, 1);
const MAX_MINOR_EXCLUSIVE: (u32, u32) = (5, 9);

const ENV_VARS: [&str; 9] = [
    "CLINGO_LIB_DIR",
    "CLINGO_INCLUDE_DIR",
    "CLINGO_STATIC",
    "CLINGO_THREADS",
    "CLINGOX_SYS_CCACHE",
    "CLINGOX_SYS_NO_VENDOR",
    "CXXSTDLIB",
    "DOCS_RS",
    "RUSTC_WRAPPER",
];

/// The directories of the vendored source that the `CMake` build reads, copied
/// whole. Together with [`SOURCE_FILES`] and the `.cmake` and `.in` files of
/// `cmake/`, they are the `include` list of `Cargo.toml`, less the licenses.
const SOURCE_DIRS: [&str; 19] = [
    "libclingo/clingo",
    "libclingo/src",
    "libgringo/gen",
    "libgringo/gringo",
    "libgringo/src",
    "libreify/reify",
    "libreify/src",
    "clasp/cmake",
    "clasp/clasp",
    "clasp/src",
    "clasp/libpotassco/cmake",
    "clasp/libpotassco/potassco",
    "clasp/libpotassco/src",
    "third_party/hopscotch-map/include",
    "third_party/optional/include",
    "third_party/ordered-map/include",
    "third_party/sparse-map/include",
    "third_party/variant/include",
    "third_party/wide-integer/math",
];

/// The single files of the vendored source that the `CMake` build reads.
const SOURCE_FILES: [&str; 10] = [
    "CMakeLists.txt",
    "libclingo/CMakeLists.txt",
    "libclingo/clingo.h",
    "libclingo/clingo.hh",
    "libclingo/clingo.map",
    "libgringo/CMakeLists.txt",
    "libreify/CMakeLists.txt",
    "clasp/CMakeLists.txt",
    "clasp/libpotassco/CMakeLists.txt",
    "third_party/CMakeLists.txt",
];

const ANDROID_COMPAT_H: &str = "\
#ifndef CLINGOX_ANDROID_COMPAT_H
#define CLINGOX_ANDROID_COMPAT_H
#include <stdlib.h>
static inline char *canonicalize_file_name(const char *path) { return realpath(path, NULL); }
#endif
";

struct Target {
    os: String,
    env: String,
    vendor: String,
    family: Vec<String>,
    features: Vec<String>,
}

impl Target {
    fn from_env() -> Self {
        let list = |name: &str| -> Vec<String> {
            env::var(name)
                .unwrap_or_default()
                .split(',')
                .map(str::to_owned)
                .collect()
        };
        Target {
            os: env::var("CARGO_CFG_TARGET_OS").unwrap_or_default(),
            env: env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default(),
            vendor: env::var("CARGO_CFG_TARGET_VENDOR").unwrap_or_default(),
            family: list("CARGO_CFG_TARGET_FAMILY"),
            features: list("CARGO_CFG_TARGET_FEATURE"),
        }
    }

    fn is_emscripten(&self) -> bool {
        self.os == "emscripten"
    }

    fn is_unix(&self) -> bool {
        self.family.iter().any(|f| f == "unix")
    }

    fn has_feature(&self, feature: &str) -> bool {
        self.features.iter().any(|f| f == feature)
    }

    /// Whether clasp is built with threads. A WASM build without the `atomics`
    /// target feature cannot run threads, so there the feature alone is not
    /// enough; this makes single-threaded the WASM default (DESIGN 9).
    fn threads(&self) -> bool {
        env::var_os("CARGO_FEATURE_THREADS").is_some()
            && (!self.is_emscripten() || self.has_feature("atomics"))
    }

    /// The C++ runtime to link, following the `cc` crate's table. `CXXSTDLIB`
    /// overrides it, and an empty value links none.
    fn cxx_stdlib(&self) -> Option<String> {
        if let Ok(lib) = env::var("CXXSTDLIB") {
            return (!lib.is_empty()).then_some(lib);
        }
        if self.env == "msvc" {
            None
        } else if self.is_emscripten() {
            Some("c++".into())
        } else if self.os == "android" {
            Some("c++_shared".into())
        } else if self.os == "netbsd" {
            // The base compiler of NetBSD is GCC with libstdc++; libc++ is not
            // installed by default.
            Some("stdc++".into())
        } else if self.vendor == "apple"
            || self.os == "freebsd"
            || self.os == "openbsd"
            || self.os == "dragonfly"
        {
            Some("c++".into())
        } else {
            Some("stdc++".into())
        }
    }
}

fn main() {
    for var in ENV_VARS {
        println!("cargo::rerun-if-env-changed={var}");
    }
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=clingo.commit");
    // Marks a build of this crate run through this script, which writes the
    // constants of `src/lib.rs` (see there).
    println!("cargo::rustc-check-cfg=cfg(clingox_sys_build)");
    println!("cargo::rustc-cfg=clingox_sys_build");

    let target = Target::from_env();

    if env::var_os("DOCS_RS").is_some() {
        // docs.rs has no C++ toolchain budget for clingo, and the docs need
        // only the committed bindings. They describe the default, vendored
        // build.
        let patches = patch_files().unwrap_or_else(|e| panic!("clingox-sys: {e}"));
        emit_metadata(
            &vendored_header_version(),
            Path::new("clingo/libclingo"),
            Path::new("clingo"),
            &Build {
                vendored: true,
                threads: target.threads(),
                patches: patch_ids(&patches),
            },
        );
        return;
    }

    let no_vendor = env::var("CLINGOX_SYS_NO_VENDOR").is_ok_and(|v| v == "1");
    let vendored = env::var_os("CARGO_FEATURE_VENDORED").is_some() && !no_vendor;
    if vendored {
        build_vendored(&target);
    } else if let Err(message) = link_system(&target) {
        panic!("clingox-sys: {message}");
    }
}

/// What was built, as `clingox_sys` and dependents' build scripts see it.
struct Build {
    vendored: bool,
    /// Whether the linked clasp has threads.
    threads: bool,
    /// The UPSTREAM-ISSUES entries patched into this build, in order.
    patches: Vec<String>,
}

fn build_vendored(target: &Target) {
    println!("cargo::rerun-if-changed=clingo");
    println!("cargo::rerun-if-changed=patches");
    let threads = target.threads();
    let on_off = |b: bool| if b { "ON" } else { "OFF" };

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let patches = patch_files().unwrap_or_else(|e| panic!("clingox-sys: {e}"));
    let source = patched_copy(
        Path::new("clingo"),
        &out_dir.join("clingo-patched"),
        &patches,
    )
    .unwrap_or_else(|e| panic!("clingox-sys: {e}"));

    let mut config = cmake::Config::new(&source);
    config
        // Release even for debug Rust builds: an unoptimised clingo is too slow
        // to test with, and nothing in it is debugged from Rust.
        .profile("Release")
        .define("CLINGO_BUILD_STATIC", "ON")
        .define("CLINGO_BUILD_SHARED", "OFF")
        .define("CLINGO_BUILD_APPS", "OFF")
        // Both default to "auto", which would pick up a host Python or Lua.
        .define("CLINGO_BUILD_WITH_PYTHON", "OFF")
        .define("CLINGO_BUILD_WITH_LUA", "OFF")
        .define("CLINGO_MANAGE_RPATH", "OFF")
        .define("CLINGO_INSTALL_LIB", "ON")
        .define("CLASP_BUILD_WITH_THREADS", on_off(threads))
        // A static build is not position independent otherwise, and Rust links
        // it into PIE executables and shared libraries.
        .define("CMAKE_POSITION_INDEPENDENT_CODE", "ON")
        // Use the committed parsers; a host bison or re2c would regenerate
        // them.
        .define("CMAKE_DISABLE_FIND_PACKAGE_BISON", "ON")
        .define("CMAKE_DISABLE_FIND_PACKAGE_RE2C", "ON");

    if target.env == "msvc" {
        config.static_crt(target.has_feature("crt-static"));
    }
    if target.is_emscripten() {
        // Rust uses native Wasm exceptions on this target. Without the same
        // model in clingo the first clingo error aborts, and -fexceptions fails
        // to link (DESIGN 9). No -flto: it breaks exception catching.
        config
            .cflag("-fwasm-exceptions")
            .cxxflag("-fwasm-exceptions");
        // Without an explicit toolchain file the cmake crate passes the host
        // compiler from CC as CMAKE_C_COMPILER. emcmake's toolchain replaces it
        // on the first configure, but every later configure then sees a changed
        // compiler, and CMake wipes the cache and drops all -D options.
        if env::var_os("CMAKE_TOOLCHAIN_FILE").is_none()
            && let Some(root) = emscripten_root()
        {
            config.define(
                "CMAKE_TOOLCHAIN_FILE",
                root.join("cmake/Modules/Platform/Emscripten.cmake"),
            );
        }
        if threads {
            config.cflag("-pthread").cxxflag("-pthread");
        }
    }
    if target.os == "android" {
        configure_android(&mut config);
    }
    forward_compiler_launcher(&mut config);

    let root = config.build();
    let lib_dir = ["lib", "lib64"]
        .iter()
        .map(|d| root.join(d))
        .find(|d| d.join(static_lib_name(target, "clingo")).exists())
        .unwrap_or_else(|| {
            panic!(
                "clingox-sys: CMake installed no libclingo under {}",
                root.display()
            )
        });

    println!("cargo::rustc-link-search=native={}", lib_dir.display());
    // On MSVC the libraries are not bundled into the rlib: bundled, they leave
    // the rlib with an empty archive symbol table (`dumpbin /linkermember`
    // lists no public symbols), and every clingo symbol is then unresolved at
    // the final link. Unbundled, `link.exe` reads the `.lib` files itself from
    // the search path above.
    let kind = if target.env == "msvc" {
        "static:-bundle"
    } else {
        "static"
    };
    for lib in LIBS {
        println!("cargo::rustc-link-lib={kind}={lib}");
    }
    link_cxx_runtime(target);
    if threads && target.is_unix() && target.os != "android" && !target.is_emscripten() {
        println!("cargo::rustc-link-lib=pthread");
    }
    if threads && clasp_needs_libatomic(&root) {
        println!("cargo::rustc-link-lib=atomic");
    }

    let version = read_header_version(&root.join("include/clingo.h"))
        .expect("the vendored clingo.h has version macros");
    emit_metadata(
        &version,
        &root.join("include"),
        &root,
        &Build {
            vendored: true,
            threads,
            patches: patch_ids(&patches),
        },
    );
}

/// The patches in `patches/`, sorted by file name, which is the order they
/// are applied in.
fn patch_files() -> Result<Vec<PathBuf>, String> {
    let dir = Path::new("patches");
    let entries = fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    let mut patches = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
            .path();
        if path.extension().is_some_and(|ext| ext == "patch") {
            patches.push(path);
        }
    }
    patches.sort();
    Ok(patches)
}

/// The UPSTREAM-ISSUES entry each patch fixes: the part of its file name
/// before the first `-` (RULES 8), as in `U1-division-traps.patch`.
fn patch_ids(patches: &[PathBuf]) -> Vec<String> {
    patches
        .iter()
        .filter_map(|p| {
            p.file_name()?
                .to_str()?
                .split('-')
                .next()
                .map(str::to_owned)
        })
        .collect()
}

/// Copies the parts of the vendored source `CMake` reads from `from` to `to`,
/// keeping the layout, and applies `patches` to the copy in order.
///
/// A file is written only when its content changes, so an unchanged source
/// keeps its timestamps and `CMake` does not rebuild it. Files in `to` that are
/// no longer part of the copy are removed.
fn patched_copy(from: &Path, to: &Path, patches: &[PathBuf]) -> Result<PathBuf, String> {
    let mut files = BTreeSet::new();
    for file in SOURCE_FILES {
        files.insert(PathBuf::from(file));
    }
    for entry in read_dir_sorted(&from.join("cmake"))? {
        let is_cmake = entry
            .extension()
            .is_some_and(|ext| ext == "cmake" || ext == "in");
        if is_cmake && entry.is_file() {
            files.insert(Path::new("cmake").join(entry.file_name().unwrap_or_default()));
        }
    }
    for dir in SOURCE_DIRS {
        collect_files(from, Path::new(dir), &mut files)?;
    }

    let mut contents = std::collections::BTreeMap::new();
    for file in &files {
        let path = from.join(file);
        let bytes = fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        contents.insert(file.clone(), bytes);
    }
    for patch in patches {
        let name = patch.display().to_string();
        let text =
            fs::read_to_string(patch).map_err(|e| format!("cannot read patch {name}: {e}"))?;
        for file_patch in parse_patch(&text).map_err(|e| format!("patch {name}: {e}"))? {
            let target = PathBuf::from(&file_patch.path);
            let original = contents.get(&target).ok_or_else(|| {
                format!(
                    "patch {name} changes {}, which is not part of the source the build reads",
                    file_patch.path
                )
            })?;
            let original = String::from_utf8(original.clone())
                .map_err(|_| format!("patch {name}: {} is not UTF-8", file_patch.path))?;
            let changed = apply_file_patch(&original, &file_patch)
                .map_err(|e| format!("patch {name} does not apply to {}: {e}", file_patch.path))?;
            contents.insert(target, changed.into_bytes());
        }
    }

    for (file, bytes) in &contents {
        let path = to.join(file);
        if fs::read(&path).is_ok_and(|old| old == *bytes) {
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        fs::write(&path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    remove_stale(to, to, &files)?;
    Ok(to.to_path_buf())
}

fn read_dir_sorted(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut entries = fs::read_dir(dir)
        .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    entries.sort();
    Ok(entries)
}

/// Adds every file under `root/dir` to `files`, as a path relative to `root`.
fn collect_files(root: &Path, dir: &Path, files: &mut BTreeSet<PathBuf>) -> Result<(), String> {
    for entry in read_dir_sorted(&root.join(dir))? {
        let relative = dir.join(entry.file_name().unwrap_or_default());
        if entry.is_dir() {
            collect_files(root, &relative, files)?;
        } else {
            files.insert(relative);
        }
    }
    Ok(())
}

/// Removes the files under `dir` whose path relative to `root` is not in
/// `keep`, left from an earlier version of the copy.
fn remove_stale(root: &Path, dir: &Path, keep: &BTreeSet<PathBuf>) -> Result<(), String> {
    let Ok(entries) = read_dir_sorted(dir) else {
        return Ok(());
    };
    for entry in entries {
        if entry.is_dir() {
            remove_stale(root, &entry, keep)?;
        } else if let Ok(relative) = entry.strip_prefix(root)
            && !keep.contains(relative)
        {
            fs::remove_file(&entry)
                .map_err(|e| format!("cannot remove {}: {e}", entry.display()))?;
        }
    }
    Ok(())
}

/// Uses the NDK's own `CMake` toolchain file when the caller did not choose
/// one, so the ABI, API level and C++ runtime match what the Rust side links.
fn configure_android(config: &mut cmake::Config) {
    // libgringo resolves #include paths with glibc's canonicalize_file_name
    // whenever __USE_GNU is defined (nongroundparser.cc:71), which bionic also
    // defines, but bionic lacks that function. glibc documents it as
    // realpath(path, NULL), so a force-included definition supplies it without
    // touching the vendored source.
    let compat = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"))
        .join("clingox_android_compat.h");
    fs::write(&compat, ANDROID_COMPAT_H).expect("OUT_DIR is writable");
    let include = format!("-include {}", compat.display());
    config.cflag(&include).cxxflag(&include);

    let target = env::var("TARGET").unwrap_or_default();
    let toolchain_var = format!("CMAKE_TOOLCHAIN_FILE_{}", target.replace('-', "_"));
    println!("cargo::rerun-if-env-changed={toolchain_var}");
    println!("cargo::rerun-if-env-changed=CMAKE_TOOLCHAIN_FILE");
    for var in [
        "ANDROID_NDK_HOME",
        "ANDROID_NDK_ROOT",
        "ANDROID_NDK",
        "ANDROID_PLATFORM",
        "CARGO_NDK_PLATFORM",
    ] {
        println!("cargo::rerun-if-env-changed={var}");
    }
    let explicit = env::var_os(&toolchain_var).or_else(|| env::var_os("CMAKE_TOOLCHAIN_FILE"));
    let ndk = ["ANDROID_NDK_HOME", "ANDROID_NDK_ROOT", "ANDROID_NDK"]
        .iter()
        .find_map(env::var_os);
    if explicit.is_none()
        && let Some(ndk) = ndk
    {
        config.define(
            "CMAKE_TOOLCHAIN_FILE",
            Path::new(&ndk).join("build/cmake/android.toolchain.cmake"),
        );
    }
    let abi = match env::var("CARGO_CFG_TARGET_ARCH")
        .unwrap_or_default()
        .as_str()
    {
        "aarch64" => "arm64-v8a",
        "arm" => "armeabi-v7a",
        "x86" => "x86",
        _ => "x86_64",
    };
    config.define("ANDROID_ABI", abi);
    if let Some(platform) = env::var("ANDROID_PLATFORM")
        .ok()
        .or_else(|| env::var("CARGO_NDK_PLATFORM").ok())
    {
        let platform = if platform.starts_with("android-") {
            platform
        } else {
            format!("android-{platform}")
        };
        config.define("ANDROID_PLATFORM", platform);
    }
    config.define("ANDROID_STL", "c++_shared");
}

/// Lets a compiler cache wrap the C++ build, alongside rustc.
///
/// sccache is picked up automatically when it already wraps rustc
/// (`RUSTC_WRAPPER`), because that is already an explicit choice on the
/// caller's part. ccache is opt-in only, by contrast: `clingox-sys` is a
/// published crate, and its build script must not start reaching outside the
/// build just because a tool happens to be on a user's `PATH`. Set
/// `CLINGOX_SYS_CCACHE=1` or `=auto` to use whatever `ccache` is found on
/// `PATH`, or to an explicit path to use that `ccache` binary; either fails
/// loudly if the requested `ccache` cannot be used, since it was asked for by
/// name. `0`, empty or unset (the default) never reaches for ccache.
///
/// Either way this sets `CMAKE_C_COMPILER_LAUNCHER`/
/// `CMAKE_CXX_COMPILER_LAUNCHER` rather than relying on a `PATH` symlink in
/// front of the real compiler (as the `/usr/lib/ccache` convention does), so
/// it reaches the host compiler, the Android NDK's clang and Emscripten's
/// `emcc`/`em++` alike, and keeps working even when `CC`/`CXX` name the real
/// compiler by an absolute path that bypasses such a symlink. `cargo xtask`
/// sets `CLINGOX_SYS_CCACHE=auto` for its own builds when ccache is present
/// (`xtask/src/util.rs`, `use_ccache_if_found`), so the workflow this crate
/// is developed under still gets the speedup (TESTING.md, "Build speed");
/// nothing changes for a downstream build unless it asks.
fn forward_compiler_launcher(config: &mut cmake::Config) {
    if let Some(wrapper) = env::var_os("RUSTC_WRAPPER") {
        let is_sccache = Path::new(&wrapper)
            .file_stem()
            .is_some_and(|s| s == "sccache");
        if is_sccache {
            config.define("CMAKE_C_COMPILER_LAUNCHER", &wrapper);
            config.define("CMAKE_CXX_COMPILER_LAUNCHER", &wrapper);
            return;
        }
    }

    let ccache = match env::var("CLINGOX_SYS_CCACHE").as_deref() {
        Ok("1" | "auto") => Some(find_on_path("ccache").unwrap_or_else(|| {
            panic!("clingox-sys: CLINGOX_SYS_CCACHE asks for ccache, but it was not found on PATH")
        })),
        Ok("" | "0") | Err(_) => None,
        Ok(path) => Some(PathBuf::from(path)),
    };
    if let Some(ccache) = ccache {
        config.define("CMAKE_C_COMPILER_LAUNCHER", &ccache);
        config.define("CMAKE_CXX_COMPILER_LAUNCHER", &ccache);
    }
}

/// Finds `name` as an executable file on `PATH`.
fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn static_lib_name(target: &Target, name: &str) -> String {
    if target.env == "msvc" {
        format!("{name}.lib")
    } else {
        format!("lib{name}.a")
    }
}

fn link_cxx_runtime(target: &Target) {
    if let Some(lib) = target.cxx_stdlib() {
        println!("cargo::rustc-link-lib={lib}");
    }
    if target.is_emscripten() && env::var("CXXSTDLIB").is_err() {
        link_emscripten_cxx_support();
    }
}

/// Reproduces what emcc's `-sDEFAULT_TO_CXX` adds to a link, with directives
/// that reach every crate linking clingox-sys; a `rustc-link-arg` would reach
/// only this package's own targets. emcc maps `-lc++` and `-lc++abi` to the
/// variant built for the exception model and optimisation level of the link.
/// Debug links also need emscripten's `libexceptions.js`, which defines the
/// stack-trace throw helper that debug libc++abi calls; emcc loads it only for
/// C++ links, but it accepts extra JS libraries as `-l<name>.js` found on the
/// library path.
fn link_emscripten_cxx_support() {
    println!("cargo::rustc-link-lib=c++abi");
    println!("cargo::rerun-if-env-changed=EMSDK");
    println!("cargo::rerun-if-env-changed=PATH");
    let Some(js) = emscripten_root().map(|root| root.join("src/lib/libexceptions.js")) else {
        println!("cargo::warning=emcc not found on PATH; debug links may miss libexceptions.js");
        return;
    };
    let dir =
        PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("emscripten-js");
    fs::create_dir_all(&dir).expect("OUT_DIR is writable");
    // Only this one file goes on the library path, so no other emscripten JS
    // library can be picked up by accident.
    fs::copy(&js, dir.join("libexceptions.js"))
        .unwrap_or_else(|e| panic!("clingox-sys: cannot copy {}: {e}", js.display()));
    println!("cargo::rerun-if-changed={}", js.display());
    println!("cargo::rustc-link-search=native={}", dir.display());
    println!("cargo::rustc-link-lib=exceptions.js");
}

/// The emscripten directory of the emcc that will link, found through `EMSDK`
/// or, for system installations, through `emcc` on PATH.
fn emscripten_root() -> Option<PathBuf> {
    if let Some(emsdk) = env::var_os("EMSDK") {
        let root = Path::new(&emsdk).join("upstream/emscripten");
        if root.join("src/lib/libexceptions.js").exists() {
            return Some(root);
        }
    }
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join("emcc"))
        .find(|emcc| emcc.is_file())
        .and_then(|emcc| fs::canonicalize(emcc).ok())
        .and_then(|emcc| emcc.parent().map(Path::to_path_buf))
        .filter(|root| root.join("src/lib/libexceptions.js").exists())
}

/// clasp's `CMake` adds `libatomic` to its thread target when 64-bit atomics
/// need it (clasp/CMakeLists.txt:66); the result is only visible in the cache.
fn clasp_needs_libatomic(root: &Path) -> bool {
    let cache = fs::read_to_string(root.join("build/CMakeCache.txt")).unwrap_or_default();
    let set = |key: &str| {
        cache.lines().any(|l| {
            l.split_once('=')
                .is_some_and(|(k, v)| k.split(':').next() == Some(key) && v == "1")
        })
    };
    !set("CLASP_HAS_WORKING_LIBATOMIC") && set("CLASP_HAS_LIBATOMIC")
}

fn link_system(target: &Target) -> Result<(), String> {
    let lib_dir = env::var_os("CLINGO_LIB_DIR").map(PathBuf::from);
    let include_dir = match env::var_os("CLINGO_INCLUDE_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => lib_dir
            .as_ref()
            .and_then(|lib| lib.parent())
            .map(|prefix| prefix.join("include"))
            .into_iter()
            .chain(["/usr/local/include", "/usr/include"].map(PathBuf::from))
            .find(|dir| dir.join("clingo.h").exists())
            .ok_or(
                "clingo.h not found; set CLINGO_INCLUDE_DIR to the directory that contains it",
            )?,
    };
    let header = include_dir.join("clingo.h");
    let version = read_header_version(&header).ok_or_else(|| {
        format!(
            "cannot read CLINGO_VERSION_* from {} (CLINGO_INCLUDE_DIR)",
            header.display()
        )
    })?;
    if version < MIN_VERSION || (version.0, version.1) >= MAX_MINOR_EXCLUSIVE {
        return Err(format!(
            "{} is clingo {}.{}.{}, but clingox-sys needs >= 5.8.1, < 5.9.0; \
             point CLINGO_INCLUDE_DIR and CLINGO_LIB_DIR at a matching installation",
            header.display(),
            version.0,
            version.1,
            version.2
        ));
    }

    if let Some(dir) = &lib_dir {
        println!("cargo::rustc-link-search=native={}", dir.display());
    }
    let threads = system_threads()?;
    let is_static = env::var("CLINGO_STATIC").is_ok_and(|v| v == "1");
    if is_static {
        for lib in LIBS {
            println!("cargo::rustc-link-lib=static={lib}");
        }
        link_cxx_runtime(target);
        if threads && target.is_unix() && target.os != "android" && !target.is_emscripten() {
            println!("cargo::rustc-link-lib=pthread");
        }
    } else {
        // The shared libclingo already contains gringo, reify, clasp and
        // potassco.
        println!("cargo::rustc-link-lib=dylib=clingo");
    }
    let root = lib_dir
        .as_deref()
        .and_then(Path::parent)
        .unwrap_or(&include_dir);
    emit_metadata(
        &version,
        &include_dir,
        root,
        &Build {
            vendored: false,
            threads,
            // A system library gets no patch (RULES 8).
            patches: Vec::new(),
        },
    );
    Ok(())
}

/// Whether a system clingo has threads, from `CLINGO_THREADS`: `1` or unset,
/// the default of clingo's `CMake` build (`CLASP_BUILD_WITH_THREADS`), or `0`.
/// Nothing in the installed library says how clasp was built.
fn system_threads() -> Result<bool, String> {
    match env::var("CLINGO_THREADS").as_deref() {
        Err(_) | Ok("1") => Ok(true),
        Ok("0") => Ok(false),
        Ok(other) => Err(format!(
            "CLINGO_THREADS is `{other}`; set it to 1 if the system clingo was built \
             with threads (CLASP_BUILD_WITH_THREADS=ON, the default) or 0 if not"
        )),
    }
}

fn vendored_header_version() -> (u32, u32, u32) {
    read_header_version(Path::new("clingo/libclingo/clingo.h"))
        .expect("the vendored clingo.h has version macros")
}

fn read_header_version(header: &Path) -> Option<(u32, u32, u32)> {
    let text = fs::read_to_string(header).ok()?;
    let macro_value = |name: &str| {
        text.lines().find_map(|line| {
            let rest = line.trim().strip_prefix("#define")?.trim_start();
            let value = rest.strip_prefix(name)?;
            value
                .starts_with(char::is_whitespace)
                .then(|| value.trim().parse::<u32>().ok())
                .flatten()
        })
    };
    Some((
        macro_value("CLINGO_VERSION_MAJOR")?,
        macro_value("CLINGO_VERSION_MINOR")?,
        macro_value("CLINGO_VERSION_REVISION")?,
    ))
}

/// Exported to dependents' build scripts as `DEP_CLINGO_*` (DESIGN 5.4), and
/// to `clingox_sys` itself as the constants of `$OUT_DIR/build_config.rs`.
fn emit_metadata(version: &(u32, u32, u32), include: &Path, root: &Path, build: &Build) {
    println!(
        "cargo::metadata=VERSION={}.{}.{}",
        version.0, version.1, version.2
    );
    println!("cargo::metadata=INCLUDE={}", include.display());
    println!("cargo::metadata=ROOT={}", root.display());
    println!("cargo::metadata=VENDORED={}", u8::from(build.vendored));
    println!("cargo::metadata=THREADS={}", u8::from(build.threads));
    println!("cargo::metadata=PATCHES={}", build.patches.join(","));

    let patches: Vec<String> = build.patches.iter().map(|p| format!("{p:?}")).collect();
    let constants = format!(
        "// Written by build.rs.\n\
         pub(crate) const HAS_THREADS: bool = {};\n\
         pub(crate) const VENDORED: bool = {};\n\
         pub(crate) const PATCHES: &[&str] = &[{}];\n",
        build.threads,
        build.vendored,
        patches.join(", ")
    );
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    fs::write(out_dir.join("build_config.rs"), constants).expect("OUT_DIR is writable");
}
