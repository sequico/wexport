//! Reading just enough of a container: how long, how big, and the first H.264 sample.
//!
//! MP4 (ISO base media) for what WhatsApp records, Matroska/WebM for the rest, Ogg for voice notes.
//! Nothing here decodes media; it only finds numbers and hands the H.264 bitstream to `media`.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::error::{Error, Result};

#[derive(Debug, Default, Clone)]
pub struct Info {
    pub duration_ms: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    /// The first H.264 access unit as Annex-B, with SPS and PPS in front, when the file is MP4/H.264.
    pub h264: Option<Vec<u8>>,
}

/// Read the numbers a record needs from a video or audio file.
pub fn probe(path: &Path) -> Result<Info> {
    let mut header = [0u8; 12];
    let mut file = File::open(path)?;
    let read = file.read(&mut header)?;
    file.seek(SeekFrom::Start(0))?;
    let bytes = &header[..read];
    if bytes.starts_with(b"OggS") {
        return ogg_info(path);
    }
    if bytes.len() >= 4 && bytes[4..8] == *b"ftyp" {
        return mp4_info(path);
    }
    if bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return ebml_info(path);
    }
    Err(Error::input("unknown media container"))
}

// ---------------------------------------------------------------- MP4

const MAX_MOOV: u64 = 16 << 20;

fn mp4_info(path: &Path) -> Result<Info> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    let moov = find_box(&mut file, length, b"moov")?
        .ok_or_else(|| Error::input("MP4 without a moov box"))?;
    let mut data = Vec::new();
    file.seek(SeekFrom::Start(moov.1))?;
    (&mut file)
        .take(moov.0.min(MAX_MOOV))
        .read_to_end(&mut data)?;

    let mut info = Info::default();
    let mut video_trak = None;
    for (kind, body) in boxes(&data) {
        if kind == *b"mvhd" {
            info.duration_ms = movie_duration(body);
        }
        if kind == *b"trak" && is_video_track(body) {
            video_trak = Some(body);
        }
    }
    let Some(trak) = video_trak else {
        return Err(Error::input("MP4 without a video track"));
    };
    if let Some(tkhd) = find_in(trak, &[b"tkhd"]) {
        let (width, height) = track_dimensions(tkhd);
        info.width = width;
        info.height = height;
    }
    if let Some(stbl) = find_in(trak, &[b"mdia", b"minf", b"stbl"]) {
        if let Some(poster) = first_h264_sample(&mut file, stbl)? {
            info.h264 = Some(poster);
        }
    }
    Ok(info)
}

/// (size, payload offset) of the first top-level box with this type.
fn find_box(file: &mut File, length: u64, wanted: &[u8; 4]) -> Result<Option<(u64, u64)>> {
    let mut offset = 0;
    while offset + 8 <= length {
        file.seek(SeekFrom::Start(offset))?;
        let mut header = [0u8; 16];
        let read = file.read(&mut header)?;
        if read < 8 {
            break;
        }
        let mut size = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as u64;
        let kind = [header[4], header[5], header[6], header[7]];
        let mut header_len = 8;
        if size == 1 {
            if read < 16 {
                break;
            }
            size = u64::from_be_bytes([
                header[8], header[9], header[10], header[11], header[12], header[13], header[14],
                header[15],
            ]);
            header_len = 16;
        } else if size == 0 {
            size = length - offset;
        }
        if size < header_len {
            break;
        }
        if kind == *wanted {
            return Ok(Some((size - header_len, offset + header_len)));
        }
        offset += size;
    }
    Ok(None)
}

/// The boxes directly inside a payload, with 64-bit sizes handled and a truncated tail ignored.
fn boxes(mut data: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    while data.len() >= 8 {
        let mut size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as u64;
        let kind = [data[4], data[5], data[6], data[7]];
        let mut header = 8;
        if size == 1 {
            if data.len() < 16 {
                break;
            }
            size = u64::from_be_bytes([
                data[8], data[9], data[10], data[11], data[12], data[13], data[14], data[15],
            ]);
            header = 16;
        } else if size == 0 {
            size = data.len() as u64;
        }
        if size < header || size > data.len() as u64 {
            break;
        }
        let (header_usize, size_usize) = (header as usize, size as usize);
        out.push((kind, &data[header_usize..size_usize]));
        data = &data[size_usize..];
    }
    out
}

/// Follow a path of box types and return the payload of the last one.
fn find_in<'a>(data: &'a [u8], path: &[&[u8; 4]]) -> Option<&'a [u8]> {
    let mut current = data;
    for wanted in path {
        let mut found = None;
        for (kind, body) in boxes(current) {
            if kind == **wanted {
                found = Some(body);
                break;
            }
        }
        current = found?;
    }
    Some(current)
}

fn is_video_track(trak: &[u8]) -> bool {
    let Some(hdlr) = find_in(trak, &[b"mdia", b"hdlr"]) else {
        return false;
    };
    hdlr.len() >= 12 && &hdlr[8..12] == b"vide"
}

fn movie_duration(mvhd: &[u8]) -> Option<i64> {
    let version = *mvhd.first()?;
    let (timescale, duration) = if version == 1 {
        let timescale = u32::from_be_bytes(mvhd.get(20..24)?.try_into().ok()?);
        let duration = u64::from_be_bytes(mvhd.get(24..32)?.try_into().ok()?);
        (timescale as u64, duration)
    } else {
        let timescale = u32::from_be_bytes(mvhd.get(12..16)?.try_into().ok()?);
        let duration = u32::from_be_bytes(mvhd.get(16..20)?.try_into().ok()?) as u64;
        (timescale as u64, duration)
    };
    if timescale == 0 || duration == 0 {
        return None;
    }
    Some((duration * 1000 / timescale).min(i64::MAX as u64) as i64)
}

/// The track header ends with the track's display width and height, in 16.16 fixed point.
fn track_dimensions(tkhd: &[u8]) -> (Option<i64>, Option<i64>) {
    let version = tkhd.first().copied().unwrap_or(0);
    let offset = if version == 1 { 88 } else { 76 };
    let Some(bytes) = tkhd.get(offset..offset + 8) else {
        return (None, None);
    };
    let width = u32::from_be_bytes(bytes[0..4].try_into().unwrap_or([0; 4])) >> 16;
    let height = u32::from_be_bytes(bytes[4..8].try_into().unwrap_or([0; 4])) >> 16;
    (
        (width > 0).then_some(width as i64),
        (height > 0).then_some(height as i64),
    )
}

/// SPS and PPS out of `avcC`, then the first sample's NAL units, all as Annex-B.
fn first_h264_sample(file: &mut File, stbl: &[u8]) -> Result<Option<Vec<u8>>> {
    let Some(stsd) = find_in(stbl, &[b"stsd"]) else {
        return Ok(None);
    };
    // stsd: version and flags (4), entry count (4), then the first sample entry.
    let entries = boxes(stsd.get(8..).unwrap_or_default());
    let Some(entry) = entries.first() else {
        return Ok(None);
    };
    if entry.0 != *b"avc1" && entry.0 != *b"avc3" {
        return Ok(None);
    }
    // Visual sample entry: 8 header + 78 fixed bytes, then the child boxes.
    let children = entry.1.get(78..).unwrap_or_default();
    let Some(avcc) = find_in(children, &[b"avcC"]) else {
        return Ok(None);
    };
    let parameter_sets = parameter_sets(avcc);
    if parameter_sets.is_empty() {
        return Ok(None);
    }
    let (chunk_offset, sample_size) = match (first_chunk(stbl), first_sample_size(stbl)) {
        (Some(offset), Some(size)) if size > 0 => (offset, size),
        _ => return Ok(None),
    };
    let mut sample = vec![0u8; sample_size.min(8 << 20)];
    file.seek(SeekFrom::Start(chunk_offset))?;
    file.read_exact(&mut sample)?;

    let mut out = Vec::new();
    for set in parameter_sets {
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(set);
    }
    out.extend_from_slice(&annex_b(&sample));
    if out.is_empty() {
        Ok(None)
    } else {
        Ok(Some(out))
    }
}

/// The SPS and PPS lists of an AVCDecoderConfigurationRecord.
fn parameter_sets(avcc: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let Some(&length_size) = avcc.get(4) else {
        return out;
    };
    if length_size & 0x03 == 0 {
        return out;
    }
    let mut offset = 5;
    let Some(&sps_count) = avcc.get(offset) else {
        return out;
    };
    offset += 1;
    for _ in 0..(sps_count & 0x1F) {
        let Some(bytes) = avcc.get(offset..offset + 2) else {
            return out;
        };
        let length = u16::from_be_bytes([bytes[0], bytes[1]]) as usize;
        let Some(set) = avcc.get(offset + 2..offset + 2 + length) else {
            return out;
        };
        out.push(set);
        offset += 2 + length;
    }
    let Some(&pps_count) = avcc.get(offset) else {
        return out;
    };
    offset += 1;
    for _ in 0..pps_count {
        let Some(bytes) = avcc.get(offset..offset + 2) else {
            return out;
        };
        let length = u16::from_be_bytes([bytes[0], bytes[1]]) as usize;
        let Some(set) = avcc.get(offset + 2..offset + 2 + length) else {
            return out;
        };
        out.push(set);
        offset += 2 + length;
    }
    out
}

/// The first sample offset: the first chunk of the first chunk group, whose offset is the sample's.
fn first_chunk(stbl: &[u8]) -> Option<u64> {
    if let Some(co64) = find_in(stbl, &[b"co64"]) {
        let count = u32::from_be_bytes(co64.get(4..8)?.try_into().ok()?);
        if count == 0 {
            return None;
        }
        return Some(u64::from_be_bytes(co64.get(8..16)?.try_into().ok()?));
    }
    let stco = find_in(stbl, &[b"stco"])?;
    let count = u32::from_be_bytes(stco.get(4..8)?.try_into().ok()?);
    if count == 0 {
        return None;
    }
    Some(u32::from_be_bytes(stco.get(8..12)?.try_into().ok()?) as u64)
}

fn first_sample_size(stbl: &[u8]) -> Option<usize> {
    let stsz = find_in(stbl, &[b"stsz"])?;
    let fixed = u32::from_be_bytes(stsz.get(4..8)?.try_into().ok()?);
    if fixed > 0 {
        return Some(fixed as usize);
    }
    let count = u32::from_be_bytes(stsz.get(8..12)?.try_into().ok()?);
    if count == 0 {
        return None;
    }
    Some(u32::from_be_bytes(stsz.get(12..16)?.try_into().ok()?) as usize)
}

/// Length-prefixed NAL units (the MP4 form) into Annex-B.
fn annex_b(sample: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(sample.len() + 16);
    let mut offset = 0;
    while offset + 4 <= sample.len() {
        let length = u32::from_be_bytes([
            sample[offset],
            sample[offset + 1],
            sample[offset + 2],
            sample[offset + 3],
        ]) as usize;
        offset += 4;
        let Some(unit) = sample.get(offset..offset + length) else {
            break;
        };
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(unit);
        offset += length;
    }
    out
}

// ---------------------------------------------------------------- Matroska

fn ebml_info(path: &Path) -> Result<Info> {
    let file = File::open(path)?;
    let length = file.metadata()?.len();
    let mut data = Vec::new();
    // The header and the metadata sit at the front, before the clusters.
    file.take(length.min(4 << 20)).read_to_end(&mut data)?;
    let mut raw = EbmlRaw::default();
    ebml_walk(&data, 0, &mut raw);
    let mut info = Info::default();
    if let Some(duration) = raw.duration {
        let scale = raw.scale.unwrap_or(1_000_000);
        info.duration_ms = Some((duration * scale as f64 / 1_000_000.0) as i64);
    }
    info.width = raw.width.map(|value| value as i64);
    info.height = raw.height.map(|value| value as i64);
    Ok(info)
}

#[derive(Default)]
struct EbmlRaw {
    scale: Option<u64>,
    duration: Option<f64>,
    width: Option<u64>,
    height: Option<u64>,
}

/// Walk the elements that matter, four containers deep; a cluster holds nothing this reads.
fn ebml_walk(data: &[u8], depth: usize, raw: &mut EbmlRaw) {
    if depth > 5 {
        return;
    }
    let mut offset = 0;
    while offset < data.len() {
        let Some((id, size, header)) = ebml_element(&data[offset..]) else {
            return;
        };
        let Some(payload) = data.get(offset + header..offset + header + size as usize) else {
            return;
        };
        match id {
            0x1853_8067 => raw.scale = uint(payload),
            0x4489 => raw.duration = float(payload),
            0xB0 => raw.width = uint(payload),
            0xBA => raw.height = uint(payload),
            0x1A45_DFA3 | 0x1549_A966 | 0x1654_AE6B | 0xAE | 0xE0 => {
                ebml_walk(payload, depth + 1, raw);
            }
            _ => {}
        }
        offset += header + size as usize;
    }
}

fn ebml_element(data: &[u8]) -> Option<(u64, u64, usize)> {
    let (id, id_len) = ebml_vint(data, true)?;
    let (size, size_len) = ebml_vint(data.get(id_len..)?, false)?;
    Some((id, size, id_len + size_len))
}

/// A variable-length integer: the marker bit belongs to the length, not the value.
fn ebml_vint(data: &[u8], keep_marker: bool) -> Option<(u64, usize)> {
    let first = *data.first()?;
    let length = first.leading_zeros() as usize + 1;
    if length > 8 || data.len() < length {
        return None;
    }
    let mut value = if keep_marker {
        first as u64
    } else {
        (first & (0xFF >> length)) as u64
    };
    for byte in &data[1..length] {
        value = (value << 8) | *byte as u64;
    }
    Some((value, length))
}

fn uint(data: &[u8]) -> Option<u64> {
    if data.is_empty() || data.len() > 8 {
        return None;
    }
    Some(
        data.iter()
            .fold(0u64, |value, byte| (value << 8) | *byte as u64),
    )
}

fn float(data: &[u8]) -> Option<f64> {
    match data.len() {
        4 => Some(f32::from_be_bytes(data.try_into().ok()?) as f64),
        8 => Some(f64::from_be_bytes(data.try_into().ok()?)),
        _ => None,
    }
}

// ---------------------------------------------------------------- Ogg

/// One Opus packet from an Ogg stream: its bytes, and whether it is audio rather than a header.
#[derive(Debug, Clone)]
pub struct OggPacket {
    pub bytes: Vec<u8>,
    pub audio: bool,
}

pub struct Ogg {
    pub channels: u8,
    pub duration_ms: Option<i64>,
    pub packets: Vec<OggPacket>,
}

/// Read an Ogg file: its Opus header, its duration from the last page, and its packets.
pub fn ogg(path: &Path) -> Result<Ogg> {
    let data = std::fs::read(path)?;
    let mut packets: Vec<OggPacket> = Vec::new();
    let mut pending: Vec<u8> = Vec::new();
    let mut channels = 1u8;
    let mut last_granule = 0u64;
    let mut offset = 0;
    while offset + 27 <= data.len() {
        if &data[offset..offset + 4] != b"OggS" {
            return Err(Error::input("Ogg stream without a page header"));
        }
        let header_type = data[offset + 5];
        let granule =
            u64::from_le_bytes(data[offset + 6..offset + 14].try_into().unwrap_or([0; 8]));
        let segments = data[offset + 26] as usize;
        let table = offset + 27;
        let Some(lacing) = data.get(table..table + segments) else {
            break;
        };
        let body_start = table + segments;
        let body_len: usize = lacing.iter().map(|value| *value as usize).sum();
        let Some(body) = data.get(body_start..body_start + body_len) else {
            break;
        };
        let mut cursor = 0;
        for length in lacing {
            let length = *length as usize;
            pending.extend_from_slice(&body[cursor..cursor + length]);
            cursor += length;
            if length < 255 {
                let packet = std::mem::take(&mut pending);
                if packet.starts_with(b"OpusHead") {
                    channels = packet.get(9).copied().unwrap_or(1).max(1);
                } else if packet.starts_with(b"OpusTags") {
                } else if !packet.is_empty() {
                    packets.push(OggPacket {
                        bytes: packet,
                        audio: true,
                    });
                }
            }
        }
        if granule != u64::MAX {
            last_granule = granule;
        }
        if header_type & 0x04 != 0 {
            break;
        }
        offset = body_start + body_len;
    }
    if packets.is_empty() {
        return Err(Error::input("Ogg stream without audio packets"));
    }
    Ok(Ogg {
        channels,
        duration_ms: (last_granule > 0).then(|| (last_granule as i64 * 1000) / 48_000),
        packets,
    })
}

// ---------------------------------------------------------------- Ogg info

fn ogg_info(path: &Path) -> Result<Info> {
    let ogg = ogg(path)?;
    Ok(Info {
        duration_ms: ogg.duration_ms,
        ..Info::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn media(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/synthetic/phone/Media")
            .join(name)
    }

    #[test]
    fn an_mp4_gives_its_duration_size_and_a_h264_sample() {
        let info = probe(&media("WhatsApp Video/VID-20261201-WA0003.mp4")).unwrap();
        assert_eq!(info.duration_ms, Some(1000));
        assert_eq!(info.width, Some(16));
        assert_eq!(info.height, Some(16));
        let sample = info.h264.expect("an H.264 sample to decode");
        assert!(sample.windows(4).any(|window| window == [0, 0, 0, 1]));
        assert!(
            sample.len() > 200,
            "the access unit carries real picture data"
        );
    }

    #[test]
    fn an_ogg_voice_note_gives_its_duration_and_packets() {
        let info = probe(&media("WhatsApp Voice Notes/PTT-20261201-WA0002.opus")).unwrap();
        let duration = info.duration_ms.expect("a duration from the granule");
        assert!((200..=500).contains(&duration), "{duration}");
        let ogg = ogg(&media("WhatsApp Voice Notes/PTT-20261201-WA0002.opus")).unwrap();
        assert!(!ogg.packets.is_empty());
        assert_eq!(ogg.channels, 1);
    }

    #[test]
    fn a_file_that_is_not_a_container_says_so() {
        let path = std::env::temp_dir().join(format!("wexport-container-{}", std::process::id()));
        std::fs::write(&path, b"not a video").unwrap();
        let error = probe(&path).unwrap_err();
        assert!(error.message.contains("unknown media container"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn lengths_and_values_are_read_big_endian() {
        let data = [0x01, 0x02, 0x03];
        assert_eq!(uint(&data), Some(0x0001_0203));
        assert_eq!(float(&[0x3F, 0x80, 0, 0]), Some(1.0));
    }
}
