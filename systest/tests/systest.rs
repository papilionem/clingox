//! Runs the ABI checks that `build.rs` generated with ctest.

// The generated code uses the C names from clingox-sys and was not written for
// clippy's style lints.
#![allow(
    non_camel_case_types,
    non_upper_case_globals,
    non_snake_case,
    clippy::all
)]

use clingox_sys::*;

include!(concat!(env!("OUT_DIR"), "/all.rs"));
