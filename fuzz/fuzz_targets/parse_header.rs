#![no_main]
//! The header parser reads bytes from a folder the user was told to copy, and the format is
//! protobuf-shaped: every field length is attacker-controlled until it is checked.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = wexport::crypt15::parse_header(data);
});
