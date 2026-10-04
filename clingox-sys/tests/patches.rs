//! The build-time patches (RULES 8): `clingox-sys/patches/` holds one
//! patch per entry of `docs/dev/UPSTREAM-ISSUES.md`, the vendored build applies
//! them to a copy of the source, and the submodule stays pristine.
//!
//! Each patch is proven by behaviour in `clingox/tests/patch_u<n>_*.rs`.
//!
//! These tests read files and run git, so they are host-only.

#![forbid(unsafe_code)]
#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail loudly on unexpected errors"
)]
#![cfg(not(any(target_os = "android", target_family = "wasm")))]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

// Same source `build.rs` uses: unit-tests the applier
// itself, including CRLF handling, without a full vendored build.
#[path = "../build/patch_applier.rs"]
mod patch_applier;

/// The patches, each named after its upstream issue (UPSTREAM-ISSUES U1, U2,
/// U14, U19, U35, U46, U47, U49, U50, U53 and U55), in the file-name order they are
/// applied in.
const EXPECTED: [&str; 11] = [
    "U1-division-traps.patch",
    "U14-emscripten-getrusage.patch",
    "U19-statistics-type-registry.patch",
    "U2-leak-symbol-table.patch",
    "U35-external-rewrite-arithmetics.patch",
    "U46-app-finish-without-threads.patch",
    "U47-ast-refcount-overflow.patch",
    "U49-range-binder-int-max.patch",
    "U50-parallel-split-leak.patch",
    "U53-reify-steps.patch",
    "U55-option-value-tables.patch",
];

fn manifest_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The patched copy of the vendored source the build compiles.
fn patched_source() -> PathBuf {
    Path::new(env!("OUT_DIR")).join("clingo-patched")
}

fn patch_names() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(manifest_dir().join("patches"))
        .expect("clingox-sys/patches exists")
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

/// The files a unified diff changes, from its `+++ b/<path>` lines.
fn patched_files(patch: &str) -> BTreeSet<String> {
    patch
        .lines()
        .filter_map(|line| line.strip_prefix("+++ b/"))
        .map(|path| path.split('\t').next().unwrap_or(path).trim().to_owned())
        .collect()
}

#[test]
fn every_patch_is_named_for_one_upstream_issue() {
    assert_eq!(patch_names(), EXPECTED);
    let issues = fs::read_to_string(manifest_dir().join("../docs/dev/UPSTREAM-ISSUES.md"))
        .expect("the upstream issues are documented");
    for name in EXPECTED {
        let number = name.split('-').next().unwrap();
        assert!(
            issues.contains(&format!("### {number}.")),
            "UPSTREAM-ISSUES.md has no entry {number}"
        );
        assert!(
            issues.contains(&format!("clingox-sys/patches/{name}")),
            "UPSTREAM-ISSUES.md does not name {name}"
        );
    }
}

#[test]
fn every_patch_is_a_small_unified_diff_of_the_vendored_source() {
    for name in EXPECTED {
        let patch = fs::read_to_string(manifest_dir().join("patches").join(name)).unwrap();
        let files = patched_files(&patch);
        assert!(!files.is_empty(), "{name} changes no file");
        for file in &files {
            assert!(
                manifest_dir().join("clingo").join(file).is_file(),
                "{name} changes {file}, which the vendored source does not have"
            );
        }
        // Small enough to read and to send upstream (RULES 8).
        let changed = patch
            .lines()
            .filter(|l| {
                (l.starts_with('+') || l.starts_with('-'))
                    && !l.starts_with("+++")
                    && !l.starts_with("---")
            })
            .count();
        assert!(changed <= 40, "{name} changes {changed} lines");
    }
}

#[test]
fn the_published_crate_carries_the_patches() {
    let manifest = include_str!("../Cargo.toml");
    assert!(
        manifest.contains("\"/patches/*.patch\""),
        "the `include` list of clingox-sys/Cargo.toml must name /patches/*.patch"
    );
}

#[test]
fn the_vendored_build_applies_every_patch_in_order() {
    if clingox_sys::VENDORED {
        assert_eq!(
            clingox_sys::PATCHES,
            [
                "U1", "U14", "U19", "U2", "U35", "U46", "U47", "U49", "U50", "U53", "U55"
            ]
        );
    } else {
        assert!(
            clingox_sys::PATCHES.is_empty(),
            "{:?}",
            clingox_sys::PATCHES
        );
    }
}

#[test]
fn the_submodule_stays_pristine() {
    let clingo = manifest_dir().join("clingo");
    let Ok(output) = Command::new("git")
        .arg("-C")
        .arg(&clingo)
        .args(["status", "--porcelain", "--untracked-files=no"])
        .output()
    else {
        return; // No git on this host: nothing to compare with.
    };
    if !output.status.success() {
        return; // Not a checkout, as in the published crate.
    }
    let changes = String::from_utf8_lossy(&output.stdout);
    assert!(
        changes.trim().is_empty(),
        "the submodule was changed:\n{changes}"
    );
}

/// Every file of the patched copy equals the submodule's, except the files a
/// patch names, which differ.
#[test]
fn the_patched_copy_differs_only_where_a_patch_says() {
    if !clingox_sys::VENDORED {
        return;
    }
    let mut named = BTreeSet::new();
    for name in EXPECTED {
        let patch = fs::read_to_string(manifest_dir().join("patches").join(name)).unwrap();
        named.extend(patched_files(&patch));
    }
    let root = patched_source();
    let mut stack = vec![root.clone()];
    let mut compared = 0;
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("the patched copy exists") {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let relative = path.strip_prefix(&root).unwrap();
            let key = relative.to_str().unwrap().replace('\\', "/");
            let original = manifest_dir().join("clingo").join(relative);
            let same = fs::read(&path).unwrap() == fs::read(&original).unwrap();
            assert_eq!(!same, named.contains(&key), "{key}");
            compared += 1;
        }
    }
    assert!(
        compared > 100,
        "the copy holds the source CMake reads: {compared} files"
    );
    for file in &named {
        assert!(root.join(file).is_file(), "{file} is missing from the copy");
    }
}
