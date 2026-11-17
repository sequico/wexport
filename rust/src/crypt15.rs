//! crypt15: the container WhatsApp writes its end-to-end encrypted backups in.
//!
//! Layout: a varint header length, a protobuf header (which carries the IV and the key type),
//! then AES-256-GCM ciphertext followed by the authentication tag and an MD5 checksum. The
//! plaintext is zlib-compressed SQLite, or a ZIP for incremental and multi-file backups.
//!
//! The key derivation and the exact tail layout are ported from the reference implementation
//! `wa-crypt-tools` (GPL-3.0), because a backup that decrypts with one and not the other would be
//! a bug nobody can find.

use std::io::Read;

use aes_gcm::aead::generic_array::typenum::U16;
use aes_gcm::aead::{Aead, KeyInit as AeadKeyInit};
use aes_gcm::aes::Aes256;
use aes_gcm::AesGcm;
use flate2::read::ZlibDecoder;
use hmac::digest::KeyInit;
use hmac::{Hmac, Mac};
use md5::Md5;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// crypt15 uses a 16-byte IV, which GCM allows but is not the default 12.
type Cipher = AesGcm<Aes256, U16>;

const BACKUP_ENCRYPTION: &[u8] = b"backup encryption";

/// What the header says about the backup, without decrypting a byte.
#[derive(Debug, Clone)]
pub struct Header {
    pub iv: Vec<u8>,
    pub app_version: String,
    /// `E2EE_ENCRYPTION_KEY`, `E2EE_PASSWORD`, `E2EE_PASSKEY`, `E2EE_DEPRECATED` or `WA_PROVIDED`.
    pub key_type: String,
    pub header_len: usize,
}

impl Header {
    pub fn is_crypt15(&self) -> bool {
        !self.iv.is_empty()
    }
}

/// Derive the AES key from the 32-byte root key.
///
/// WhatsApp wraps the root key in a nested HMAC loop (`wa_hmacsha256_loop.java`); for the local
/// backup only one iteration runs, with the counter byte appended.
pub fn derive_key(root: &[u8; 32]) -> [u8; 32] {
    let mut private =
        <Hmac<Sha256> as KeyInit>::new_from_slice(&[0u8; 32]).expect("hmac takes any key");
    Mac::update(&mut private, root);
    let private_key = private.finalize().into_bytes();

    let mut hasher =
        <Hmac<Sha256> as KeyInit>::new_from_slice(&private_key).expect("hmac takes any key");
    Mac::update(&mut hasher, BACKUP_ENCRYPTION);
    Mac::update(&mut hasher, &[1u8]);
    let out = hasher.finalize().into_bytes();
    let mut key = [0u8; 32];
    key.copy_from_slice(&out);
    key
}

pub fn parse_header(bytes: &[u8]) -> Result<Header> {
    let (size, mut offset) = read_varint(bytes, 0)?;
    let size = size as usize;
    let end = offset
        .checked_add(size)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| Error::input("header length runs past the end of the file"))?;
    let proto = &bytes[offset..end];
    offset = end;

    let mut iv = Vec::new();
    let mut crypt14_iv = Vec::new();
    let mut app_version = String::new();
    let mut key_type = 0u64;
    for (number, value) in fields(proto) {
        match (number, value) {
            (3, Field::Bytes(inner)) => iv = message_bytes(inner, 1).unwrap_or_default(),
            (2, Field::Bytes(inner)) => crypt14_iv = message_bytes(inner, 5).unwrap_or_default(),
            (4, Field::Bytes(inner)) => {
                for (field, value2) in fields(inner) {
                    if field == 1 {
                        if let Field::Bytes(text) = value2 {
                            app_version = String::from_utf8_lossy(text).into_owned();
                        }
                    }
                }
            }
            (6, Field::Varint(kind)) => key_type = kind,
            _ => {}
        }
    }
    if iv.is_empty() {
        iv = crypt14_iv;
    }
    Ok(Header {
        iv,
        app_version,
        key_type: key_name(key_type).to_string(),
        header_len: offset,
    })
}

fn key_name(value: u64) -> &'static str {
    match value {
        0 => "WA_PROVIDED",
        1 => "E2EE_DEPRECATED",
        2 => "E2EE_PASSWORD",
        3 => "E2EE_ENCRYPTION_KEY",
        4 => "E2EE_PASSKEY",
        _ => "UNKNOWN",
    }
}

enum Field<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
}

fn fields(mut input: &[u8]) -> Vec<(u64, Field<'_>)> {
    let mut out = Vec::new();
    while !input.is_empty() {
        let Ok((key, used)) = read_varint(input, 0) else {
            break;
        };
        input = &input[used..];
        let number = key >> 3;
        let wire = key & 0x7;
        match wire {
            0 => {
                let Ok((value, used)) = read_varint(input, 0) else {
                    break;
                };
                input = &input[used..];
                out.push((number, Field::Varint(value)));
            }
            2 => {
                let Ok((len, used)) = read_varint(input, 0) else {
                    break;
                };
                let len = len as usize;
                if used + len > input.len() {
                    break;
                }
                out.push((number, Field::Bytes(&input[used..used + len])));
                input = &input[used + len..];
            }
            1 => {
                if input.len() < 8 {
                    break;
                }
                input = &input[8..];
            }
            5 => {
                if input.len() < 4 {
                    break;
                }
                input = &input[4..];
            }
            _ => break,
        }
    }
    out
}

fn message_bytes(input: &[u8], wanted: u64) -> Option<Vec<u8>> {
    for (number, value) in fields(input) {
        if number == wanted {
            if let Field::Bytes(bytes) = value {
                return Some(bytes.to_vec());
            }
        }
    }
    None
}

fn read_varint(input: &[u8], mut offset: usize) -> Result<(u64, usize)> {
    let mut value = 0u64;
    for shift in (0..=63).step_by(7) {
        let byte = *input
            .get(offset)
            .ok_or_else(|| Error::input("file ended inside the header length"))?;
        offset += 1;
        value |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, offset));
        }
    }
    Err(Error::input("header length varint is too long"))
}

/// Decrypt a crypt15 file and return the payload bytes, decompressed.
pub fn decrypt(bytes: &[u8], header: &Header, root: &[u8; 32]) -> Result<Vec<u8>> {
    if header.iv.len() != 16 {
        return Err(Error::input(format!(
            "the header carries a {}-byte IV, not 16: this is not a crypt15 backup",
            header.iv.len()
        )));
    }
    let body = &bytes[header.header_len..];
    if body.len() < 32 {
        return Err(Error::input(
            "the backup is truncated before its authentication tag",
        ));
    }
    let (data, tag, checksum) = (
        &body[..body.len() - 32],
        &body[body.len() - 32..body.len() - 16],
        &body[body.len() - 16..],
    );

    let mut digest = Md5::new();
    digest.update(data);
    digest.update(tag);
    // The checksum covers the whole file except its own 16 bytes: header included. That is how the
    // reference implementation accumulates it, so a header-only difference must not flip the layout.
    let mut whole = Md5::new();
    whole.update(&bytes[..bytes.len() - 16]);
    let single_file = whole.finalize().as_slice() == checksum;
    let _ = digest.finalize();

    let cipher = Cipher::new_from_slice(&derive_key(root)).map_err(|_| Error::input("bad key"))?;
    let nonce = aes_gcm::Nonce::<U16>::from_slice(&header.iv);
    // A GCM message is ciphertext followed by its 16-byte tag. Single-file backups keep the tag
    // before the MD5 checksum; multi-file ones carry no checksum and the tag is the last 16 bytes.
    let sealed = if single_file {
        &body[..body.len() - 16]
    } else {
        body
    };
    let plain = cipher
        .decrypt(nonce, sealed)
        .map_err(|_| Error::key("the key does not open this backup, or the backup is corrupt"))?;

    let mut data = Vec::new();
    ZlibDecoder::new(plain.as_slice())
        .read_to_end(&mut data)
        .map_err(|error| Error::input(format!("the payload is not zlib data: {error}")))?;
    if data.starts_with(b"PK\x03\x04") {
        return largest_database(&data);
    }
    if !data.starts_with(b"SQLite format 3\0") {
        return Err(Error::input(
            "the decrypted payload is not a SQLite database",
        ));
    }
    Ok(data)
}

fn largest_database(zip_bytes: &[u8]) -> Result<Vec<u8>> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes))
        .map_err(|error| Error::input(format!("the ZIP payload cannot be read: {error}")))?;
    let mut best: Option<(u64, String)> = None;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| Error::input(format!("ZIP entry unreadable: {error}")))?;
        if entry.name().ends_with(".db") {
            let size = entry.size();
            if best
                .as_ref()
                .map(|(current, _)| size > *current)
                .unwrap_or(true)
            {
                best = Some((size, entry.name().to_string()));
            }
        }
    }
    let (_, name) = best.ok_or_else(|| Error::input("the ZIP payload holds no .db file"))?;
    let mut file = archive
        .by_name(&name)
        .map_err(|error| Error::input(format!("ZIP entry {name} unreadable: {error}")))?;
    let mut out = Vec::new();
    file.read_to_end(&mut out)
        .map_err(|error| Error::input(format!("ZIP entry {name} unreadable: {error}")))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_derivation_matches_the_reference() {
        let mut root = [0u8; 32];
        let hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        for index in 0..32 {
            root[index] = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).unwrap();
        }
        let derived = derive_key(&root);
        let hex: String = derived.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(
            hex,
            "d44f47a5e7d390c365ea62ece489a5418f2ef9907e0383cd49766d1b819769dc"
        );
    }
}

#[cfg(test)]
mod fixture_tests {
    use super::*;

    const FIXTURE: &[u8] =
        include_bytes!("../../py/tests/fixtures/synthetic/phone/Databases/msgstore.db.crypt15");

    #[test]
    fn header_of_the_fixture_is_read() {
        let header = parse_header(FIXTURE).unwrap();
        let iv: String = header.iv.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(iv, "bd75ec327fb184c2b0fb38bbf0d26049");
        assert_eq!(header.key_type, "E2EE_ENCRYPTION_KEY");
        assert_eq!(header.app_version, "2.26.34.7");
    }
}

#[cfg(test)]
mod decrypt_tests {
    use super::*;

    const FIXTURE: &[u8] =
        include_bytes!("../../py/tests/fixtures/synthetic/phone/Databases/msgstore.db.crypt15");
    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn root() -> [u8; 32] {
        let mut out = [0u8; 32];
        for index in 0..32 {
            out[index] = u8::from_str_radix(&KEY[index * 2..index * 2 + 2], 16).unwrap();
        }
        out
    }

    #[test]
    fn the_fixture_decrypts_to_sqlite() {
        let header = parse_header(FIXTURE).unwrap();
        let plain = decrypt(FIXTURE, &header, &root()).unwrap();
        assert!(plain.starts_with(b"SQLite format 3\0"));
        assert!(plain.len() > 4096);
    }

    #[test]
    fn a_wrong_key_is_refused() {
        let header = parse_header(FIXTURE).unwrap();
        let mut wrong = root();
        wrong[0] ^= 0xff;
        assert!(decrypt(FIXTURE, &header, &wrong).is_err());
    }
}
