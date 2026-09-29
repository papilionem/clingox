#!/bin/bash
# Cross-builds the clingox-sys and clingox test executables for an Android
# target with the runner's NDK and copies them, with libc++_shared.so, to OUT.
# It sets the variables `cargo xtask test android` sets for x86_64, which
# xtask itself only knows for that one ABI.
#
# usage: android-build-tests.sh TARGET CLANG_TRIPLE LIB_TRIPLE OUT
#   TARGET        rust target, e.g. aarch64-linux-android
#   CLANG_TRIPLE  prefix of the NDK's clang wrappers, e.g. armv7a-linux-androideabi
#   LIB_TRIPLE    directory of the NDK sysroot libraries, e.g. arm-linux-androideabi
set -euo pipefail

target=${1:?usage: android-build-tests.sh TARGET CLANG_TRIPLE LIB_TRIPLE OUT}
clang_triple=${2:?}
lib_triple=${3:?}
out=${4:?}
api=${ANDROID_PLATFORM:-24}

ndk_llvm="${ANDROID_NDK_HOME:?}/toolchains/llvm/prebuilt/linux-x86_64"
bin="$ndk_llvm/bin"
env_target=${target//-/_}
upper=${env_target^^}

export "CARGO_TARGET_${upper}_LINKER=$bin/${clang_triple}${api}-clang"
export "CC_${env_target}=$bin/${clang_triple}${api}-clang"
export "CXX_${env_target}=$bin/${clang_triple}${api}-clang++"
export "AR_${env_target}=$bin/llvm-ar"
export ANDROID_PLATFORM="$api"

json=$(mktemp)
cargo test --locked --no-run --target "$target" -p clingox-sys -p clingox \
    --message-format=json-render-diagnostics > "$json"
mkdir -p "$out"
jq -r 'select(.reason == "compiler-artifact" and .profile.test == true and .executable != null) | .executable' \
    "$json" | while read -r exe; do cp "$exe" "$out/"; done
cp "$ndk_llvm/sysroot/usr/lib/${lib_triple}/libc++_shared.so" "$out/"
ls -l "$out"
