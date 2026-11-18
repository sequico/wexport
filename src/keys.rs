//! The three accepted key forms: 64 hex digits, a key file, or a screenshot of the digits.

use std::path::Path;
use std::process::Command;

use crate::error::{Error, Result};

const IMAGE_SUFFIXES: [&str; 7] = ["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff"];

pub fn normalize_hex(text: &str) -> Option<[u8; 32]> {
    let digits: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != ':')
        .collect();
    if digits.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&digits[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// The 32-byte root key for whatever the user handed over.
pub fn load(spec: &str) -> Result<[u8; 32]> {
    if let Some(key) = normalize_hex(spec) {
        return Ok(key);
    }
    let path = Path::new(spec);
    if !path.is_file() {
        return Err(Error::key(format!(
            "key not found: {spec}\nGive the 64 hex digits, a screenshot of them, or encrypted_backup.key."
        )));
    }
    let suffix = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if IMAGE_SUFFIXES.contains(&suffix.as_str()) {
        return read_from_image(path);
    }
    let bytes = std::fs::read(path)?;
    if let Ok(text) = std::str::from_utf8(&bytes) {
        if let Some(key) = normalize_hex(text) {
            return Ok(key);
        }
    }
    if bytes.len() >= 32 {
        // encrypted_backup.key is a serialised byte[]; the root key is its last 32 bytes.
        let mut key = [0u8; 32];
        key.copy_from_slice(&bytes[bytes.len() - 32..]);
        return Ok(key);
    }
    Err(Error::key(format!(
        "{} is not a usable key file: it holds {} bytes",
        path.display(),
        bytes.len()
    )))
}

fn read_from_image(path: &Path) -> Result<[u8; 32]> {
    let output = Command::new("tesseract")
        .arg(path)
        .arg("stdout")
        .output()
        .map_err(|_| {
            Error::dependency(format!(
                "tesseract not found on PATH, needed to read the key from a screenshot.\n{}\nOr type the 64 hex digits instead of passing an image.",
                crate::error::install_hints("tesseract")
            ))
        })?;
    if !output.status.success() {
        return Err(Error::key(format!(
            "tesseract could not read {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    if let Some(key) = normalize_hex(&text) {
        return Ok(key);
    }
    // Digits are often read together with other text: take the first run of 64 hex characters.
    let cleaned: String = text
        .chars()
        .filter(|c| c.is_ascii_hexdigit() || c.is_whitespace())
        .collect();
    if let Some(key) = normalize_hex(&cleaned) {
        return Ok(key);
    }
    Err(Error::key(
        "could not read 64 hex digits from that image.\nCrop the screenshot to the key itself, or type the digits.",
    ))
}
