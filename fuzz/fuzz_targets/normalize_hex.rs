#![no_main]
//! The key from a typed string, which arrives from a terminal, a file or an OCR result.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = wexport::keys::normalize_hex(text);
    }
});
