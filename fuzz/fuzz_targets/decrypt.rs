#![no_main]
//! The whole decrypt path on arbitrary bytes: header, checksum, GCM, zlib, ZIP.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(header) = wexport::crypt15::parse_header(data) {
        let mut bytes = data.to_vec();
        let key = [0u8; 32];
        let _ = wexport::crypt15::decrypt_to(&mut bytes, &header, &key, &mut std::io::sink());
    }
});
