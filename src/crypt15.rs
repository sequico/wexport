//! crypt15: the container WhatsApp writes its end-to-end encrypted backups in.
//!
//! Layout: a varint header length, a protobuf header (which carries the IV and the key type),
//! then AES-256-GCM ciphertext followed by the authentication tag and an MD5 checksum. The
//! plaintext is zlib-compressed SQLite, or a ZIP for incremental and multi-file backups.
//!
//! The key derivation and the exact tail layout are ported from the reference implementation
//! `wa-crypt-tools` (GPL-3.0), because a backup that decrypts with one and not the other would be
//! a bug nobody can find.

use std::io::{Read, Write};

use aes_gcm::aead::generic_array::typenum::U16;
use aes_gcm::aead::{AeadInPlace, KeyInit as AeadKeyInit};
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
    /// Only an e2ee key header carries the field this IV comes from; a crypt14 header does not.
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
    let mut app_version = String::new();
    let mut key_type = 0u64;
    for (number, value) in fields(proto) {
        match (number, value) {
            (3, Field::Bytes(inner)) => iv = message_bytes(inner, 1).unwrap_or_default(),
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
                // A corrupt length near 2^64 would otherwise wrap and point past the end.
                let Ok(len) = usize::try_from(len) else {
                    break;
                };
                let Some(end) = used.checked_add(len) else {
                    break;
                };
                if end > input.len() {
                    break;
                }
                out.push((number, Field::Bytes(&input[used..end])));
                input = &input[end..];
            }
            1 => {
                let Some(rest) = input.get(8..) else {
                    break;
                };
                input = rest;
            }
            5 => {
                let Some(rest) = input.get(4..) else {
                    break;
                };
                input = rest;
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

fn read_varint(input: &[u8], offset: usize) -> Result<(u64, usize)> {
    let tail = input
        .get(offset..)
        .ok_or_else(|| Error::input("file ended inside the header length"))?;
    let mut value = 0u64;
    // Ten groups of seven bits are the whole u64; a longer run is not a length this file can carry.
    for (index, byte) in tail.iter().take(10).enumerate() {
        value |= u64::from(byte & 0x7f) << (index * 7);
        if byte & 0x80 == 0 {
            return Ok((value, offset + index + 1));
        }
    }
    Err(if tail.len() < 10 {
        Error::input("file ended inside the header length")
    } else {
        Error::input("header length varint is too long")
    })
}

/// Decrypt a crypt15 file in place and write its payload into `dest`.
///
/// The buffer is the file itself: decryption happens in place, so the peak is the compressed backup
/// and never a second copy of it; the payload leaves through `dest` a block at a time.
pub fn decrypt_to<W: Write>(
    bytes: &mut [u8],
    header: &Header,
    root: &[u8; 32],
    dest: &mut W,
) -> Result<()> {
    if header.iv.len() != 16 {
        return Err(Error::input(format!(
            "the header carries a {}-byte IV, not 16: this is not a crypt15 backup",
            header.iv.len()
        )));
    }
    let body_start = header.header_len;
    if bytes.len() < body_start + 32 {
        return Err(Error::input(
            "the backup is truncated before its authentication tag",
        ));
    }
    // The checksum covers the whole file except its own 16 bytes. A single-file backup keeps it
    // after the tag; a multi-file one has no checksum and its tag is the last 16 bytes.
    let mut whole = Md5::new();
    whole.update(&bytes[..bytes.len() - 16]);
    let single_file = whole.finalize().as_slice() == &bytes[bytes.len() - 16..];
    let end = if single_file {
        bytes.len() - 16
    } else {
        bytes.len()
    };

    let mut tag = [0u8; 16];
    tag.copy_from_slice(&bytes[end - 16..end]);

    let cipher = Cipher::new_from_slice(&derive_key(root)).map_err(|_| Error::input("bad key"))?;
    let nonce = aes_gcm::Nonce::<U16>::from_slice(&header.iv);
    cipher
        .decrypt_in_place_detached(
            nonce,
            &[],
            &mut bytes[body_start..end - 16],
            aes_gcm::Tag::<U16>::from_slice(&tag),
        )
        .map_err(|_| Error::key("the key does not open this backup, or the backup is corrupt"))?;

    // In place means the plaintext replaced the ciphertext: nothing was copied to get here. It is
    // still zlib: the sniff happens on the decompressed stream, which leaves through `dest`.
    let mut decoder = ZlibDecoder::new(&bytes[body_start..end - 16]);
    let mut magic = [0u8; 16];
    decoder
        .read_exact(&mut magic)
        .map_err(|error| Error::input(format!("the payload is not zlib data: {error}")))?;
    if magic.starts_with(b"SQLite format 3\0") {
        dest.write_all(&magic)?;
        std::io::copy(&mut decoder, dest)
            .map_err(|error| Error::input(format!("the payload is not zlib data: {error}")))?;
        return Ok(());
    }
    if magic.starts_with(b"PK\x03\x04") {
        // A ZIP needs random access, so this shape is the one case held whole: it is the compressed
        // payload, and it carries several databases that have to be compared.
        let mut zipped = magic.to_vec();
        decoder
            .read_to_end(&mut zipped)
            .map_err(|error| Error::input(format!("the payload is not zlib data: {error}")))?;
        dest.write_all(&largest_database(&zipped)?)?;
        return Ok(());
    }
    Err(Error::input(
        "the decrypted payload is not a SQLite database",
    ))
}

fn largest_database(zip_bytes: &[u8]) -> Result<Vec<u8>> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes))
        .map_err(|error| Error::input(format!("the ZIP payload cannot be read: {error}")))?;
    let mut best: Option<(u64, String)> = None;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| Error::input(format!("ZIP entry unreadable: {error}")))?;
        let name = entry
            .name()
            .map_err(|error| Error::input(format!("ZIP entry name unreadable: {error}")))?
            .to_string();
        if name.ends_with(".db") {
            let size = entry.size();
            if best
                .as_ref()
                .map(|(current, _)| size > *current)
                .unwrap_or(true)
            {
                best = Some((size, name));
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
    use proptest::prelude::*;

    const FIXTURE: &[u8] =
        include_bytes!("../tests/fixtures/synthetic/phone/Databases/msgstore.db.crypt15");
    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn root() -> [u8; 32] {
        crate::keys::normalize_hex(KEY).expect("the fixture key is 64 hex digits")
    }

    /// A crypt15 header built by hand, for the round-trip property.
    fn header_bytes(iv: &[u8; 16], version: &str, key_type: u8) -> Vec<u8> {
        let mut e2ee = vec![0x0a, 16];
        e2ee.extend_from_slice(iv);
        let mut metadata = vec![0x0a, version.len() as u8];
        metadata.extend_from_slice(version.as_bytes());
        let mut proto = vec![0x1a, e2ee.len() as u8];
        proto.extend(e2ee);
        proto.extend([0x22, metadata.len() as u8]);
        proto.extend(metadata);
        proto.extend([0x30, key_type]);
        let mut out = vec![proto.len() as u8];
        out.extend(proto);
        out
    }

    proptest! {
        #[test]
        fn any_bytes_are_refused_without_panicking(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
            let parsed = parse_header(&bytes);
            if let Ok(header) = parsed {
                // Whatever it read must be self-consistent, never a panic halfway.
                prop_assert!(header.header_len <= bytes.len());
            }
        }

        #[test]
        fn a_well_formed_header_round_trips(iv in any::<[u8; 16]>(), key_type in 0u8..5) {
            let bytes = header_bytes(&iv, "2.26.34.7", key_type);
            let header = parse_header(&bytes).unwrap();
            prop_assert_eq!(header.iv.as_slice(), iv.as_slice());
            prop_assert_eq!(header.key_type.as_str(), key_name(key_type as u64));
            prop_assert_eq!(header.app_version.as_str(), "2.26.34.7");
            prop_assert!(header.is_crypt15());
        }
    }

    #[test]
    fn key_derivation_matches_the_reference() {
        let derived = derive_key(&root());
        let hex: String = derived.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(
            hex,
            "d44f47a5e7d390c365ea62ece489a5418f2ef9907e0383cd49766d1b819769dc"
        );
    }

    #[test]
    fn header_of_the_fixture_is_read() {
        let header = parse_header(FIXTURE).unwrap();
        let iv: String = header.iv.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(iv, "bd75ec327fb184c2b0fb38bbf0d26049");
        assert_eq!(header.key_type, "E2EE_ENCRYPTION_KEY");
        assert_eq!(header.app_version, "2.26.34.7");
    }

    #[test]
    fn a_wa_provided_header_is_not_mistaken_for_crypt15() {
        // Field 2 (the key WhatsApp provides) carries an IV of its own on crypt14 backups; only
        // field 3 means end-to-end encryption, so the presence of field 2 must not answer for it.
        let mut header = vec![0x14, 0x12, 0x12, 0x2a, 0x10];
        header.extend_from_slice(&[7u8; 16]);
        let header = parse_header(&header).unwrap();
        assert_eq!(header.key_type, "WA_PROVIDED");
        assert!(!header.is_crypt15());
    }

    #[test]
    fn the_fixture_decrypts_to_sqlite() {
        let header = parse_header(FIXTURE).unwrap();
        let mut bytes = FIXTURE.to_vec();
        let mut plain = Vec::new();
        decrypt_to(&mut bytes, &header, &root(), &mut plain).unwrap();
        assert!(plain.starts_with(b"SQLite format 3\0"));
        assert!(plain.len() > 4096);
    }

    #[test]
    fn a_wrong_key_is_refused() {
        let header = parse_header(FIXTURE).unwrap();
        let mut wrong = root();
        wrong[0] ^= 0xff;
        let mut bytes = FIXTURE.to_vec();
        assert!(decrypt_to(&mut bytes, &header, &wrong, &mut Vec::new()).is_err());
    }

    #[test]
    fn every_key_type_has_its_name() {
        assert_eq!(key_name(0), "WA_PROVIDED");
        assert_eq!(key_name(1), "E2EE_DEPRECATED");
        assert_eq!(key_name(2), "E2EE_PASSWORD");
        assert_eq!(key_name(3), "E2EE_ENCRYPTION_KEY");
        assert_eq!(key_name(4), "E2EE_PASSKEY");
        assert_eq!(key_name(9), "UNKNOWN");
    }

    #[test]
    fn the_wire_types_a_header_can_carry_are_stepped_over() {
        // A fixed64 field (wire 1) and a fixed32 one (wire 5), then the real e2ee field: a parser
        // that does not know their sizes would read the IV from the wrong offset.
        let mut proto = Vec::new();
        proto.push(0x09); // field 1, wire 1
        proto.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        proto.push(0x1d); // field 3, wire 5
        proto.extend_from_slice(&[9, 9, 9, 9]);
        proto.extend([0x1a, 18, 0x0a, 16]);
        proto.extend_from_slice(&[7u8; 16]);

        let mut bytes = vec![proto.len() as u8];
        bytes.extend(proto);
        let header = parse_header(&bytes).unwrap();
        assert_eq!(header.iv, vec![7u8; 16]);
        assert!(header.is_crypt15());
    }

    #[test]
    fn a_field_length_that_wraps_the_counter_is_refused_not_a_panic() {
        // Field 3 (bytes) with a length of 2^64-1: ten 0xff bytes. A plain `used + len` wraps round
        // to a small number and the slice that follows panics on a corrupt backup.
        let mut proto = vec![0x1a];
        proto.extend_from_slice(&[0xff; 9]);
        proto.push(0x7f);
        proto.extend_from_slice(&[0u8; 16]);
        let mut bytes = vec![proto.len() as u8];
        bytes.extend(proto);
        let header = parse_header(&bytes).unwrap();
        assert!(
            header.iv.is_empty(),
            "the field is dropped, the parse carries on"
        );
    }

    #[test]
    fn a_field_that_claims_more_bytes_than_it_has_stops_the_parse() {
        // Field 3 (bytes) declares 40 bytes with 3 present, and a valid IV follows the lie.
        let mut proto = vec![0x1a, 40, 1, 2, 3];
        proto.extend([0x1a, 18, 0x0a, 16]);
        proto.extend_from_slice(&[7u8; 16]);
        let mut bytes = vec![proto.len() as u8];
        bytes.extend(proto);

        let header = parse_header(&bytes).unwrap();
        assert!(
            header.iv.is_empty(),
            "the parser trusts the length, not the bytes"
        );
        assert!(!header.is_crypt15());
    }

    #[test]
    fn a_body_shorter_than_a_tag_is_refused_before_decrypting() {
        let bytes = header_bytes(&[7u8; 16], "2.26.34.7", 3);
        let header = parse_header(&bytes).unwrap();
        let mut short = bytes.clone();
        short.extend_from_slice(&[0u8; 31]);
        let error = decrypt_to(&mut short, &header, &root(), &mut Vec::new()).unwrap_err();
        assert!(error.message.contains("truncated"));
    }

    #[test]
    fn the_largest_database_inside_a_zip_is_the_one_returned() {
        use std::io::{Cursor, Write};
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("wa.db", options).unwrap();
        writer.write_all(b"small").unwrap();
        writer.start_file("msgstore.db", options).unwrap();
        writer.write_all(b"the bigger database").unwrap();
        let archive = writer.finish().unwrap().into_inner();

        let plain = largest_database(&archive).unwrap();
        assert_eq!(plain, b"the bigger database");
    }

    #[test]
    fn a_zip_without_a_database_is_an_error() {
        use std::io::Cursor;
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("notes.txt", options).unwrap();
        let archive = writer.finish().unwrap().into_inner();
        assert!(largest_database(&archive).is_err());
    }

    #[test]
    fn a_field_larger_than_a_hundred_and_twenty_seven_bytes_is_read_correctly() {
        // Its length takes two varint bytes, which is where "used + len" and "used * len" differ.
        let mut proto = vec![0x0a, 0xc8, 0x01]; // field 1, 200 bytes, two-byte length
        proto.extend(std::iter::repeat_n(0u8, 200));
        proto.extend([0x1a, 18, 0x0a, 16]);
        proto.extend_from_slice(&[7u8; 16]);
        let mut bytes = varint(proto.len());
        bytes.extend(proto);
        let header = parse_header(&bytes).unwrap();
        assert_eq!(header.iv, vec![7u8; 16]);
    }

    fn varint(mut value: usize) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                out.push(byte);
                return out;
            }
            out.push(byte | 0x80);
        }
    }

    #[test]
    fn exactly_a_tag_of_body_is_not_called_truncated() {
        let bytes = header_bytes(&[7u8; 16], "2.26.34.7", 3);
        let header = parse_header(&bytes).unwrap();
        let mut exact = bytes.clone();
        exact.extend_from_slice(&[0u8; 32]);
        let error = decrypt_to(&mut exact, &header, &root(), &mut Vec::new()).unwrap_err();
        assert!(
            !error.message.contains("truncated"),
            "the boundary itself is not truncation: {}",
            error.message
        );
    }

    #[test]
    fn two_databases_of_the_same_size_keep_the_first() {
        use std::io::{Cursor, Write};
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("first.db", options).unwrap();
        writer.write_all(b"aaaa").unwrap();
        writer.start_file("second.db", options).unwrap();
        writer.write_all(b"bbbb").unwrap();
        let archive = writer.finish().unwrap().into_inner();
        assert_eq!(largest_database(&archive).unwrap(), b"aaaa");
    }

    #[test]
    fn a_whole_backup_can_be_built_and_read_back() {
        // Encrypt a ZIP payload the way WhatsApp does, then read it: this covers the multi-file
        // layout (no MD5 tail) from header to payload.
        use aes_gcm::aead::AeadInPlace;
        use std::io::{Cursor, Write};

        let mut zipped = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        zipped.start_file("msgstore.db", options).unwrap();
        zipped.write_all(b"SQLite format 3\0 first").unwrap();
        zipped.start_file("wa.db", options).unwrap();
        zipped.write_all(b"smaller").unwrap();
        let zipped = zipped.finish().unwrap().into_inner();

        let compressed = {
            use flate2::write::ZlibEncoder;
            use flate2::Compression;
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(&zipped).unwrap();
            encoder.finish().unwrap()
        };

        let mut message = compressed;
        let cipher = Cipher::new_from_slice(&derive_key(&root())).unwrap();
        let iv = [9u8; 16];
        let nonce = aes_gcm::Nonce::<U16>::from_slice(&iv);
        let tag = cipher
            .encrypt_in_place_detached(nonce, &[], &mut message)
            .unwrap();
        message.extend_from_slice(&tag);

        let mut file = header_bytes(&iv, "2.26.34.7", 3);
        file.extend_from_slice(&message);

        let header = parse_header(&file).unwrap();
        let mut plain = Vec::new();
        decrypt_to(&mut file, &header, &root(), &mut plain).unwrap();
        assert!(plain.starts_with(b"SQLite format 3\0"));
        assert_eq!(plain, b"SQLite format 3\0 first");
    }
}
