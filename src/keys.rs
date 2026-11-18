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
    // Anything that is not a hex digit is not the key, and a two-byte character would split the
    // pairs below down the middle.
    if digits.len() != 64 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
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
    // The same `~` expansion the folder and -o accept; the prompt takes what a person types.
    let path = crate::discover::expand(Path::new(spec));
    if !path.is_file() {
        return Err(Error::key(format!(
            "key not found: {}\nGive the 64 hex digits, a screenshot of them, or encrypted_backup.key.",
            path.display()
        )));
    }
    let suffix = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if IMAGE_SUFFIXES.contains(&suffix.as_str()) {
        return read_from_image(&path);
    }
    let bytes = std::fs::read(&path)?;
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
    if let Some(key) = normalize_hex(&text).or_else(|| key_among_text(&text)) {
        return Ok(key);
    }
    Err(Error::key(
        "could not read 64 hex digits from that image.\nCrop the screenshot to the key itself, or type the digits.",
    ))
}

/// The key as it comes out of OCR: a label, line breaks, often a 4×4 grid.
///
/// The text is cut into runs of hex characters, spaces, dashes and colons; a run that holds exactly
/// 64 hex digits is the key. Filtering the whole page instead would mix the letters of the label
/// into the digits — "backup" contributes four of them.
fn key_among_text(text: &str) -> Option<[u8; 32]> {
    text.split(|character: char| {
        !(character.is_ascii_hexdigit()
            || character.is_whitespace()
            || character == '-'
            || character == ':')
    })
    .filter_map(|run| {
        let digits: String = run
            .chars()
            .filter(|character| character.is_ascii_hexdigit())
            .collect();
        (digits.len() == 64).then_some(digits)
    })
    .find_map(|digits| normalize_hex(&digits))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_that_is_not_ascii_hex_is_refused_not_a_panic() {
        // 64 bytes, but one of them is a two-byte character at an odd offset: slicing the string
        // without checking would split it down the middle.
        let mut text = String::from("a");
        text.push('è');
        text.push_str(&"0".repeat(61));
        assert_eq!(text.len(), 64);
        assert_eq!(normalize_hex(&text), None);
    }

    #[test]
    fn the_fixture_key_is_accepted_in_the_shapes_a_person_types_it() {
        assert!(
            normalize_hex("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .is_some()
        );
        assert!(normalize_hex(
            "0123 4567 89ab cdef 0123 4567 89ab cdef 0123 4567 89ab cdef 0123 4567 89ab cdef"
        )
        .is_some());
        assert!(normalize_hex(
            "0123-4567-89ab-cdef-0123-4567-89ab-cdef-0123-4567-89ab-cdef-0123-4567-89ab-cdef"
        )
        .is_some());
        assert!(normalize_hex("0123456789abcdef").is_none());
        assert!(
            normalize_hex("zz23456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .is_none()
        );
    }

    #[test]
    fn a_failing_tesseract_is_an_error_even_when_it_prints_a_key() {
        let _guard = path_lock();
        let root = std::env::temp_dir().join(format!("wexport-ocr-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let script = root.join("tesseract");
        // Prints a perfectly good key, then fails: the exit status is what decides.
        std::fs::write(
            &script,
            "#!/bin/sh\necho '0123 4567 89ab cdef 0123 4567 89ab cdef 0123 4567 89ab cdef 0123 4567 89ab cdef'\nexit 1\n",
        )
        .unwrap();
        make_executable(&script);
        let image = root.join("key.png");
        std::fs::write(&image, b"not really a png").unwrap();

        let original = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", root.display(), original));
        let result = load(image.to_str().unwrap());
        std::env::set_var("PATH", original);

        assert!(result.is_err(), "a non-zero tesseract must not be trusted");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_key_is_found_among_the_text_tesseract_returns() {
        let _guard = path_lock();
        let root = std::env::temp_dir().join(format!("wexport-ocr-mixed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let script = root.join("tesseract");
        std::fs::write(
            &script,
            "#!/bin/sh\necho 'WhatsApp backup key:'\necho '0123 4567 89ab cdef 0123 4567 89ab cdef 0123 4567 89ab cdef 0123 4567 89ab cdef'\n",
        )
        .unwrap();
        make_executable(&script);
        let image = root.join("key.png");
        std::fs::write(&image, b"not really a png").unwrap();

        let original = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", root.display(), original));
        let result = load(image.to_str().unwrap());
        std::env::set_var("PATH", original);

        assert_eq!(
            result.unwrap(),
            normalize_hex("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .unwrap()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// The OCR tests change PATH, which is process-wide: cargo runs them as threads.
    fn path_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    fn make_executable(path: &std::path::Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
        }
    }

    #[test]
    fn a_key_file_may_hold_the_raw_key_or_the_digits() {
        let root = std::env::temp_dir().join(format!("wexport-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let raw = root.join("encrypted_backup.key");
        std::fs::write(&raw, [7u8; 32]).unwrap();
        assert_eq!(load(raw.to_str().unwrap()).unwrap(), [7u8; 32]);

        let text = root.join("key.txt");
        std::fs::write(
            &text,
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\n",
        )
        .unwrap();
        assert_eq!(
            load(text.to_str().unwrap()).unwrap(),
            normalize_hex("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .unwrap()
        );

        let _ = std::fs::remove_dir_all(root);
    }
}

#[cfg(test)]
mod text_tests {
    use super::*;

    #[test]
    fn the_key_is_taken_from_one_run_and_not_from_the_hex_letters_of_a_label() {
        let key = "0123 4567 89ab cdef 0123 4567 89ab cdef 0123 4567 89ab cdef 0123 4567 89ab cdef";
        let expected = normalize_hex(key).unwrap();

        // The label, and the 4×4 grid as WhatsApp shows it: the run may cross a line break.
        let grid =
            "0123 4567 89ab cdef\n0123 4567 89ab cdef\n0123 4567 89ab cdef\n0123 4567 89ab cdef";
        assert_eq!(
            key_among_text(&format!("backup key:\n{grid}\n")),
            Some(expected)
        );
        assert_eq!(key_among_text(&format!("Your key: {key}")), Some(expected));

        // 63 digits is not the key, and neither is 65.
        let short: String = key.chars().take(key.len() - 1).collect();
        assert_eq!(key_among_text(&short), None);
        assert_eq!(key_among_text(&format!("{key}0")), None);
    }
}
