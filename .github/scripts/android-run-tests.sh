#!/bin/bash
# Pushes the prebuilt Android test executables in DIR to the connected device
# and runs each, the way `cargo xtask test android` does (xtask itself only
# knows the x86_64 emulator and builds in the same run).
#
# usage: android-run-tests.sh DIR [ABI]
#   DIR  holds the test executables and libc++_shared.so
#   ABI  an ABI the device must list in ro.product.cpu.abilist (default
#        arm64-v8a); an emulator with ARM translation lists more than its own
set -euo pipefail

dir=${1:?usage: android-run-tests.sh DIR [ABI]}
want_abi=${2:-arm64-v8a}
device_dir=/data/local/tmp/clingox-tests

abilist=$(adb shell getprop ro.product.cpu.abilist | tr -d '\r')
if ! tr ',' '\n' <<< "$abilist" | grep -qx "$want_abi"; then
    echo "the connected device supports $abilist, not $want_abi" >&2
    exit 1
fi

adb shell rm -rf "$device_dir"
adb shell mkdir -p "$device_dir"
adb push "$dir/libc++_shared.so" "$device_dir"

failed=()
count=0
for exe in "$dir"/*; do
    name=$(basename "$exe")
    [ "$name" = libc++_shared.so ] && continue
    count=$((count + 1))
    adb push "$exe" "$device_dir/$name"
    # `adb shell` has not always forwarded the exit status, so the marker decides.
    out=$(adb shell "cd $device_dir && chmod +x ./$name && LD_LIBRARY_PATH=$device_dir ./$name; echo xtask-exit=\$?" | tr -d '\r')
    printf '%s\n' "$out"
    adb shell rm -f "$device_dir/$name"
    printf '%s\n' "$out" | grep -qx 'xtask-exit=0' || failed+=("$name")
done
adb shell rm -rf "$device_dir"

if [ "$count" -eq 0 ]; then
    echo "no test executables in $dir" >&2
    exit 1
fi
if [ "${#failed[@]}" -ne 0 ]; then
    echo "android: failed: ${failed[*]}" >&2
    exit 1
fi
echo "android: $count test executables passed"
