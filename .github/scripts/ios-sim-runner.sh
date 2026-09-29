#!/bin/bash
# Cargo target runner for aarch64-apple-ios-sim: runs one test executable inside
# the booted simulator named by IOS_SIM_UDID and returns its exit status, so
# `cargo test --target aarch64-apple-ios-sim` works like a native run.
#
# `simctl spawn` starts a plain command-line executable in the simulator's
# runtime, which is all these test executables are; no app bundle is needed.
# It passes only variables that carry the SIMCTL_CHILD_ prefix to the child, so
# the ones cargo and the tests read at run time are forwarded that way.
set -euo pipefail

: "${IOS_SIM_UDID:?boot a simulator and export its UDID first}"

while IFS='=' read -r name value; do
    case "$name" in
        CARGO_* | RUST_* | CLINGO*)
            export "SIMCTL_CHILD_${name}=${value}"
            ;;
    esac
done < <(env)

exec xcrun simctl spawn "$IOS_SIM_UDID" "$@"
